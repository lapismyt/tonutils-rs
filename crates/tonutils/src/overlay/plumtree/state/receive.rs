//! Plumtree receive-path validation, forwarding, and FEC reassembly.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use crate::{
    adnl::{now_i32, tl::AdnlTlPublicKey as TlPublicKey},
    overlay::{OverlayError, PeerId, protocol_stats, tl::OverlayCertificate},
    tl::TlInt256 as Int256,
};

use super::super::PartInput;
use super::super::wire::{
    self, MAX_PLUMTREE_BROADCAST_SIZE, PLUMTREE_SIMPLE_TREE_INDEX, fec_packet, new_fec_decoder,
};
use super::{
    Broadcast, BroadcastKey, FecBroadcast, FecMessage, IHaveMessage, MAX_ACTIVE_REPAIR_QUERIES,
    MissingPart, PartState, PayloadOrigin, ProcessOutcome, RepairKey, RepairRequest,
    SimpleBroadcast, SimpleMessage, State,
};

struct FecInsert {
    flags: i32,
    full_data_hash: Int256,
    full_data_size: u32,
    part_size: u32,
    part: PartState,
}

struct FecFields {
    full_data_size: u32,
    part_size: u32,
    part_index: u32,
    tree: u32,
    broadcast_id: Int256,
    key: BroadcastKey,
    payload_key: RepairKey,
}

enum PartDisposition {
    New,
    Duplicate,
    Conflict,
}

impl State {
    fn verify_signed(
        &self,
        src: &TlPublicKey,
        certificate: &OverlayCertificate,
        size: u32,
        signature: &[u8],
        to_sign: &[u8],
    ) -> Result<(), OverlayError> {
        let source_key = wire::source_key(src)?;
        let source_hash = wire::source_hash(src)?;
        wire::check_certificate(
            self.overlay,
            source_hash,
            certificate,
            size,
            true,
            now_i32(),
        )?;
        wire::verify(source_key, to_sign, signature)
    }

    fn insert_fec_part(&mut self, key: BroadcastKey, insert: FecInsert) {
        let FecInsert {
            flags,
            full_data_hash,
            full_data_size,
            part_size,
            part,
        } = insert;
        let part_index = part.part_index;
        self.broadcasts.entry(key).or_insert_with(|| {
            Broadcast::Fec(FecBroadcast {
                flags,
                full_data_hash,
                full_data_size,
                part_size,
                source_count: full_data_size.div_ceil(part_size),
                first_seen_at: Instant::now(),
                delivered: false,
                decode_failed: false,
                decoder: None,
                decoder_parts: HashSet::new(),
                parts: HashMap::new(),
            })
        });
        if let Some(Broadcast::Fec(state)) = self.broadcasts.get_mut(&key) {
            state.parts.insert(part_index, part);
        }
        self.stats.parts_received = self.stats.parts_received.saturating_add(1);
        protocol_stats::record_plumtree_part_received();
    }

    fn insert_simple(
        &mut self,
        key: BroadcastKey,
        payload_key: RepairKey,
        flags: i32,
        data: &[u8],
        part: PartState,
    ) {
        self.broadcasts.insert(
            key,
            Broadcast::Simple(SimpleBroadcast {
                flags,
                payload: data.to_vec(),
                first_seen_at: Instant::now(),
                part,
            }),
        );
        self.remove_missing(&payload_key);
        self.delivered.insert(key, Instant::now());
        self.stats.parts_received = self.stats.parts_received.saturating_add(1);
        protocol_stats::record_plumtree_part_received();
        self.stats.completed_broadcasts = self.stats.completed_broadcasts.saturating_add(1);
        protocol_stats::record_plumtree_broadcast_completed();
    }

    fn validate_fec_fields(
        &self,
        flags: i32,
        full_data_hash: &Int256,
        full_data_size: i32,
        part_index: i32,
        tree_index: i32,
        data_len: usize,
    ) -> Result<FecFields, OverlayError> {
        let Ok(full_data_size) = u32::try_from(full_data_size) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "negative Plumtree full data size".to_owned(),
            ));
        };
        if full_data_size == 0
            || full_data_size as usize > MAX_PLUMTREE_BROADCAST_SIZE
            || data_len == 0
        {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree FEC fields".to_owned(),
            ));
        }
        let part_size = u32::try_from(data_len).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("Plumtree FEC part is too large".to_owned())
        })?;
        if part_size != full_data_size.div_ceil(self.config.k) {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree part size".to_owned(),
            ));
        }
        let broadcast_id = wire::fec_id(
            flags,
            full_data_hash.clone(),
            full_data_size.cast_signed(),
            part_size.cast_signed(),
        );
        wire::check_control_fields(&broadcast_id, part_index, tree_index, &self.config)?;
        let Ok(part_index) = u32::try_from(part_index) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "negative Plumtree part index".to_owned(),
            ));
        };
        let Ok(tree) = u32::try_from(tree_index) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "negative Plumtree tree index".to_owned(),
            ));
        };
        let key = broadcast_id.0;
        Ok(FecFields {
            full_data_size,
            part_size,
            part_index,
            tree,
            broadcast_id,
            key,
            payload_key: (key, part_index, tree),
        })
    }

    fn fec_part_disposition(
        &self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
        data_hash: &Int256,
    ) -> PartDisposition {
        let Some(known) = self
            .broadcasts
            .get(&key)
            .and_then(Broadcast::part_by_index)
            .and_then(|parts| parts.get(&part_index))
        else {
            return PartDisposition::New;
        };
        if known.tree_index == tree && known.data_hash == *data_hash {
            PartDisposition::Duplicate
        } else {
            PartDisposition::Conflict
        }
    }

    fn delivered_without_state(&self, key: BroadcastKey) -> bool {
        !self.broadcasts.contains_key(&key) && self.delivered.contains_key(&key)
    }

    fn source_is_ineligible(
        &self,
        from: PeerId,
        tree: u32,
        origin: PayloadOrigin,
        payload_key: RepairKey,
    ) -> Result<bool, OverlayError> {
        if let PayloadOrigin::RepairResponse(expected) = origin
            && expected != payload_key
        {
            return Err(OverlayError::InvalidPlumtreePayload(
                "Plumtree repair response key mismatch".to_owned(),
            ));
        }
        Ok(!self.receive_broadcasts
            || matches!(origin, PayloadOrigin::Push) && !self.is_eager(tree, from))
    }

    fn decode_fec_part(
        &mut self,
        key: BroadcastKey,
        part_index: u32,
        data: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, OverlayError> {
        let mut payload = None;
        if let Some(Broadcast::Fec(state)) = self.broadcasts.get_mut(&key)
            && !state.delivered
            && !state.decode_failed
        {
            if state.decoder.is_none() {
                state.decoder = Some(new_fec_decoder(state.full_data_size, state.part_size)?);
            }
            if state.decoder_parts.insert(part_index)
                && let Some(decoder) = state.decoder.as_mut()
            {
                let packet = fec_packet(part_index, data, state.source_count);
                if let Some(decoded) = decoder.decode(packet) {
                    let decoded_hash = wire::sha256(&decoded);
                    if decoded_hash != state.full_data_hash
                        || decoded.len() != state.full_data_size as usize
                    {
                        state.decode_failed = true;
                        state.decoder = None;
                        return Err(OverlayError::InvalidPlumtreePayload(
                            "Plumtree decoded data hash mismatch".to_owned(),
                        ));
                    }
                    state.delivered = true;
                    state.decoder = None;
                    self.delivered.insert(key, Instant::now());
                    self.stats.completed_broadcasts =
                        self.stats.completed_broadcasts.saturating_add(1);
                    protocol_stats::record_plumtree_broadcast_completed();
                    payload = Some(decoded);
                }
            }
        }
        Ok(payload)
    }

    fn register_missing_repair(
        &mut self,
        from: PeerId,
        missing_key: super::RepairKey,
        tree: u32,
    ) -> ProcessOutcome {
        self.note_control_received();
        let immediate = self.local_eager_limit() == 0
            || self
                .slots
                .get(tree as usize)
                .is_none_or(|slot| slot.eager.is_empty());
        let created = !self.missing.contains_key(&missing_key);
        if created {
            self.evict_missing_parts();
            self.missing.insert(
                missing_key,
                MissingPart {
                    targets: vec![from],
                    repair_at: Instant::now() + self.config.repair_timeout,
                },
            );
            self.missing_order.push_back(missing_key);
        } else if let Some(missing) = self.missing.get_mut(&missing_key) {
            missing.targets.push(from);
        }

        let mut outcome = ProcessOutcome::default();
        if immediate && self.active_repair_queries < MAX_ACTIVE_REPAIR_QUERIES {
            self.activate_repair(missing_key, from);
            outcome.repairs.push(RepairRequest {
                key: missing_key,
                peer: from,
            });
        }
        outcome
    }

    pub(super) fn process_simple(
        &mut self,
        from: PeerId,
        message: SimpleMessage,
        origin: PayloadOrigin,
    ) -> Result<ProcessOutcome, OverlayError> {
        let SimpleMessage {
            flags,
            timestamp,
            src,
            certificate,
            broadcast_id,
            tree_index,
            data,
            signature,
        } = message;
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(&broadcast_id, 0, tree_index, &self.config)?;
        let tree = u32::try_from(tree_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree tree index".to_owned())
        })?;
        if data.is_empty() || data.len() > MAX_PLUMTREE_BROADCAST_SIZE {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree simple payload size".to_owned(),
            ));
        }
        let key = broadcast_id.0;
        let payload_key = (key, 0, tree);
        if self.source_is_ineligible(from, tree, origin, payload_key)? {
            return Ok(self.duplicate_payload(from, tree, key, 0, tree));
        }
        if self.delivered_without_state(key) {
            return Ok(ProcessOutcome::default());
        }
        let data_hash = wire::sha256(&data);
        if let Some(existing) = self.broadcasts.get(&key) {
            match existing {
                Broadcast::Simple(state)
                    if state.flags == flags
                        && state.part.data_hash == data_hash
                        && state.part.source_key == src =>
                {
                    return Ok(self.duplicate_payload(from, tree, key, 0, tree));
                }
                _ => return Err(OverlayError::PlumtreeBroadcastConflict),
            }
        }

        let data_size = u32::try_from(data.len()).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("Plumtree simple payload is too large".to_owned())
        })?;
        self.verify_signed(
            &src,
            &certificate,
            data_size,
            &signature,
            &wire::simple_to_sign(
                broadcast_id.clone(),
                timestamp,
                tree_index,
                data_size.cast_signed(),
                data_hash.clone(),
            ),
        )?;
        let part = PartState {
            part_index: 0,
            tree_index: tree,
            timestamp,
            data_size,
            data_hash,
            source_key: src,
            certificate,
            signature,
            data: data.clone(),
            full_sends: 0,
            advertised_to: HashSet::new(),
            full_sent_to: HashSet::new(),
        };
        self.insert_simple(key, payload_key, flags, &data, part);

        let mut actions = Vec::new();
        self.promote_eager(tree, from, true);
        actions.push(self.useful_action(from, key, 0, tree));
        self.forward_payload(key, 0, tree, Some(from), &mut actions);
        Ok(ProcessOutcome {
            payload: Some(data),
            actions,
            repairs: Vec::new(),
        })
    }

    pub(super) fn process_fec(
        &mut self,
        from: PeerId,
        message: FecMessage,
        origin: PayloadOrigin,
    ) -> Result<ProcessOutcome, OverlayError> {
        let FecMessage {
            flags,
            timestamp,
            src,
            certificate,
            full_data_hash,
            full_data_size,
            part_index,
            tree_index,
            data,
            signature,
        } = message;
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        let FecFields {
            full_data_size,
            part_size,
            part_index,
            tree,
            broadcast_id,
            key,
            payload_key,
        } = self.validate_fec_fields(
            flags,
            &full_data_hash,
            full_data_size,
            part_index,
            tree_index,
            data.len(),
        )?;
        if self.source_is_ineligible(from, tree, origin, payload_key)? {
            return Ok(self.duplicate_payload(from, tree, key, part_index, tree));
        }
        if self.delivered_without_state(key) {
            return Ok(ProcessOutcome::default());
        }
        if let Some(existing) = self.broadcasts.get(&key) {
            match existing {
                Broadcast::Fec(state)
                    if state.flags == flags
                        && state.full_data_hash == full_data_hash
                        && state.full_data_size == full_data_size
                        && state.part_size == part_size => {}
                _ => return Err(OverlayError::PlumtreeBroadcastConflict),
            }
        }
        let data_hash = wire::sha256(&data);
        match self.fec_part_disposition(key, part_index, tree, &data_hash) {
            PartDisposition::Duplicate => {
                return Ok(self.duplicate_payload(from, tree, key, part_index, tree));
            }
            PartDisposition::Conflict => return Err(OverlayError::PlumtreeBroadcastConflict),
            PartDisposition::New => {}
        }

        self.verify_signed(
            &src,
            &certificate,
            full_data_size,
            &signature,
            &wire::fec_to_sign(
                broadcast_id.clone(),
                timestamp,
                part_index.cast_signed(),
                tree_index,
                part_size.cast_signed(),
                data_hash.clone(),
            ),
        )?;

        let part = PartState::from(PartInput {
            part_index,
            tree_index: tree,
            timestamp,
            data_size: part_size,
            data_hash,
            source_key: src,
            certificate,
            signature,
            data: data.clone(),
        });
        self.insert_fec_part(
            key,
            FecInsert {
                flags,
                full_data_hash,
                full_data_size,
                part_size,
                part,
            },
        );
        self.remove_missing(&payload_key);

        self.promote_eager(tree, from, true);
        let mut actions = vec![self.useful_action(from, key, part_index, tree)];
        self.forward_payload(key, part_index, tree, Some(from), &mut actions);

        let payload = self.decode_fec_part(key, part_index, data)?;
        Ok(ProcessOutcome {
            payload,
            actions,
            repairs: Vec::new(),
        })
    }

    pub(super) fn process_ihave(
        &mut self,
        from: PeerId,
        message: IHaveMessage,
    ) -> Result<ProcessOutcome, OverlayError> {
        let IHaveMessage {
            broadcast_id,
            timestamp,
            part_index,
            tree_index,
            src,
            certificate,
            payload_timestamp,
            data_size,
            data_hash,
            signature,
        } = message;
        if !self.receive_broadcasts {
            return Ok(ProcessOutcome::default());
        }
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(&broadcast_id, part_index, tree_index, &self.config)?;
        let Ok(part_index) = u32::try_from(part_index) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "negative Plumtree part index".to_owned(),
            ));
        };
        let Ok(tree) = u32::try_from(tree_index) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "negative Plumtree tree index".to_owned(),
            ));
        };
        let key = broadcast_id.0;
        if !self.broadcasts.contains_key(&key) && self.delivered.contains_key(&key) {
            return Ok(ProcessOutcome::default());
        }
        if self
            .broadcasts
            .get(&key)
            .and_then(Broadcast::part_by_index)
            .is_some_and(|known| known.contains_key(&part_index))
        {
            return Ok(ProcessOutcome::default());
        }
        wire::check_timestamp(payload_timestamp, wire::unix_seconds())?;
        let Ok(data_size) = u32::try_from(data_size) else {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree IHave data size".to_owned(),
            ));
        };
        if data_size == 0 || data_size as usize > MAX_PLUMTREE_BROADCAST_SIZE {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree IHave data size".to_owned(),
            ));
        }
        if tree != PLUMTREE_SIMPLE_TREE_INDEX {
            let max_part_size = (MAX_PLUMTREE_BROADCAST_SIZE as u32).div_ceil(self.config.k);
            if data_size > max_part_size {
                return Err(OverlayError::InvalidPlumtreePayload(
                    "too large Plumtree IHave FEC part".to_owned(),
                ));
            }
        }
        let missing_key = (key, part_index, tree);
        if let Some(missing) = self.missing.get(&missing_key)
            && (missing.targets.len() >= self.config.max_repair_targets as usize
                || missing.targets.contains(&from))
        {
            return Ok(ProcessOutcome::default());
        }

        let to_sign = if tree == PLUMTREE_SIMPLE_TREE_INDEX {
            wire::simple_to_sign(
                broadcast_id.clone(),
                payload_timestamp,
                tree_index,
                data_size.cast_signed(),
                data_hash,
            )
        } else {
            wire::fec_to_sign(
                broadcast_id.clone(),
                payload_timestamp,
                part_index.cast_signed(),
                tree_index,
                data_size.cast_signed(),
                data_hash,
            )
        };
        self.verify_signed(&src, &certificate, data_size, &signature, &to_sign)?;

        Ok(self.register_missing_repair(from, missing_key, tree))
    }

    pub(super) fn process_prune(
        &mut self,
        from: PeerId,
        broadcast_id: &Int256,
        timestamp: f64,
        part_index: i32,
        tree_index: i32,
    ) -> Result<ProcessOutcome, OverlayError> {
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(broadcast_id, part_index, tree_index, &self.config)?;
        let part_index = u32::try_from(part_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree part index".to_owned())
        })?;
        let tree = u32::try_from(tree_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree tree index".to_owned())
        })?;
        self.note_control_received();
        if let Some(part) = self.part(broadcast_id.0, part_index, tree)
            && part.full_sent_to.contains(&from)
        {
            self.note_eager_active(from);
            self.remove_eager(tree, from);
        }
        Ok(ProcessOutcome::default())
    }

    pub(super) fn process_useful(
        &mut self,
        from: PeerId,
        broadcast_id: &Int256,
        timestamp: f64,
        part_index: i32,
        tree_index: i32,
    ) -> Result<ProcessOutcome, OverlayError> {
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(broadcast_id, part_index, tree_index, &self.config)?;
        let part_index = u32::try_from(part_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree part index".to_owned())
        })?;
        let tree = u32::try_from(tree_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree tree index".to_owned())
        })?;
        self.note_control_received();
        let sent = self
            .part(broadcast_id.0, part_index, tree)
            .is_some_and(|part| part.full_sent_to.contains(&from));
        if sent {
            self.note_eager_active(from);
            let was_pending = self
                .slots
                .get_mut(tree as usize)
                .is_some_and(|slot| slot.pending_feedback.remove(&from).is_some());
            if was_pending {
                self.promote_eager(tree, from, false);
            }
        }
        Ok(ProcessOutcome::default())
    }

    pub(super) fn process_stats_push(
        &mut self,
        record: &crate::overlay::tl::OverlayPlumtreeStatsRecord,
    ) {
        let tree_ok = (0..self.config.tree_slots).contains(&record.tree_index.cast_unsigned());
        let eager_ok = record.eager_0.len() <= self.config.eager_limit as usize
            && record.eager_index.len() <= self.config.eager_limit as usize;
        if tree_ok && eager_ok {
            self.note_control_received();
        }
    }
}
