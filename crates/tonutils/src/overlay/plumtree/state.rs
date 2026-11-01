//! Per-overlay Plumtree topology, duplicate suppression, and repair state.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use raptorq::Decoder;

use crate::{
    adnl::tl::AdnlTlPublicKey as TlPublicKey,
    overlay::{
        OverlayError, OverlayId, PeerId, protocol_stats,
        tl::{OverlayBroadcast, OverlayCertificate},
    },
    tl::TlInt256 as Int256,
};

use super::{
    PartInput, PlumtreeBroadcastId, PlumtreeConfig, PlumtreeStats,
    wire::{self, MAX_PLUMTREE_BROADCAST_SIZE, PLUMTREE_FEC_TREE_OFFSET},
};

mod receive;
mod repair;
mod topology;

#[cfg(test)]
mod tests;

const PENDING_FEEDBACK_TTL: Duration = Duration::from_secs(5);
const EAGER_PEER_INACTIVITY_TTL: Duration = Duration::from_secs(30);
const EAGER_PEER_MAX_SENT_WITHOUT_ACTIVITY: u32 = 50;
const MAX_PENDING_REPAIR_PARTS: usize = 1024;
const MAX_ACTIVE_REPAIR_QUERIES: usize = 512;
const BROADCAST_LIFETIME: Duration = Duration::from_secs(20);

type BroadcastKey = [u8; 32];
pub(crate) type RepairKey = (BroadcastKey, u32, u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PayloadOrigin {
    Push,
    RepairResponse(RepairKey),
}

#[derive(Debug)]
pub(crate) enum Action {
    Send { peer: PeerId, wire: Vec<u8> },
    Repair(RepairRequest),
}

#[derive(Debug, Default)]
pub(crate) struct ProcessOutcome {
    pub payload: Option<Vec<u8>>,
    pub actions: Vec<Action>,
    pub repairs: Vec<RepairRequest>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RepairRequest {
    pub key: RepairKey,
    pub peer: PeerId,
}

pub(super) struct SimpleMessage {
    pub flags: i32,
    pub timestamp: f64,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub broadcast_id: Int256,
    pub tree_index: i32,
    pub data: Vec<u8>,
    pub signature: Vec<u8>,
}

pub(super) struct FecMessage {
    pub flags: i32,
    pub timestamp: f64,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub full_data_hash: Int256,
    pub full_data_size: i32,
    pub part_index: i32,
    pub tree_index: i32,
    pub data: Vec<u8>,
    pub signature: Vec<u8>,
}

pub(super) struct IHaveMessage {
    pub broadcast_id: Int256,
    pub timestamp: f64,
    pub part_index: i32,
    pub tree_index: i32,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub payload_timestamp: f64,
    pub data_size: i32,
    pub data_hash: Int256,
    pub signature: Vec<u8>,
}

impl TryFrom<OverlayBroadcast> for SimpleMessage {
    type Error = OverlayError;

    fn try_from(broadcast: OverlayBroadcast) -> Result<Self, Self::Error> {
        let OverlayBroadcast::PlumtreeSimple {
            flags,
            timestamp,
            src,
            certificate,
            broadcast_id,
            tree_index,
            data,
            signature,
        } = broadcast
        else {
            return Err(invalid_message("simple"));
        };
        Ok(Self {
            flags,
            timestamp: timestamp.to_f64(),
            src,
            certificate,
            broadcast_id,
            tree_index,
            data,
            signature,
        })
    }
}

impl TryFrom<OverlayBroadcast> for FecMessage {
    type Error = OverlayError;

    fn try_from(broadcast: OverlayBroadcast) -> Result<Self, Self::Error> {
        let OverlayBroadcast::PlumtreeFec {
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
        } = broadcast
        else {
            return Err(invalid_message("FEC"));
        };
        Ok(Self {
            flags,
            timestamp: timestamp.to_f64(),
            src,
            certificate,
            full_data_hash,
            full_data_size,
            part_index,
            tree_index,
            data,
            signature,
        })
    }
}

impl TryFrom<OverlayBroadcast> for IHaveMessage {
    type Error = OverlayError;

    fn try_from(broadcast: OverlayBroadcast) -> Result<Self, Self::Error> {
        let OverlayBroadcast::PlumtreeIHave {
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
        } = broadcast
        else {
            return Err(invalid_message("IHave"));
        };
        Ok(Self {
            broadcast_id,
            timestamp: timestamp.to_f64(),
            part_index,
            tree_index,
            src,
            certificate,
            payload_timestamp: payload_timestamp.to_f64(),
            data_size,
            data_hash,
            signature,
        })
    }
}

fn invalid_message(kind: &str) -> OverlayError {
    OverlayError::InvalidPlumtreePayload(format!("unexpected Plumtree {kind} message"))
}

pub(crate) struct FecStart {
    pub id: PlumtreeBroadcastId,
    pub flags: i32,
    pub full_data_hash: Int256,
    pub full_data_size: u32,
    pub part_size: u32,
    pub source_count: u32,
    pub parts: Vec<PartInput>,
}

#[derive(Default)]
struct Slot {
    eager: HashSet<PeerId>,
    pending_feedback: HashMap<PeerId, Instant>,
}

struct PeerActivity {
    last_active_at: Instant,
    sent_since_active: u32,
}

impl Default for PeerActivity {
    fn default() -> Self {
        Self {
            last_active_at: Instant::now(),
            sent_since_active: 0,
        }
    }
}

struct PartState {
    part_index: u32,
    tree_index: u32,
    timestamp: f64,
    data_size: u32,
    data_hash: Int256,
    source_key: TlPublicKey,
    certificate: OverlayCertificate,
    signature: Vec<u8>,
    data: Vec<u8>,
    full_sends: u32,
    advertised_to: HashSet<PeerId>,
    full_sent_to: HashSet<PeerId>,
}

struct SimpleBroadcast {
    flags: i32,
    payload: Vec<u8>,
    first_seen_at: Instant,
    part: PartState,
}

struct FecBroadcast {
    flags: i32,
    full_data_hash: Int256,
    full_data_size: u32,
    part_size: u32,
    source_count: u32,
    first_seen_at: Instant,
    delivered: bool,
    decode_failed: bool,
    decoder: Option<Decoder>,
    decoder_parts: HashSet<u32>,
    parts: HashMap<u32, PartState>,
}

enum Broadcast {
    Simple(SimpleBroadcast),
    Fec(FecBroadcast),
}

struct MissingPart {
    targets: Vec<PeerId>,
    repair_at: Instant,
}

pub(crate) struct State {
    config: PlumtreeConfig,
    overlay: OverlayId,
    local_source: Option<Int256>,
    local_peer: Option<PeerId>,
    receive_broadcasts: bool,
    known_peers: Vec<PeerId>,
    slots: Vec<Slot>,
    peer_activity: HashMap<PeerId, PeerActivity>,
    broadcasts: HashMap<BroadcastKey, Broadcast>,
    delivered: HashMap<BroadcastKey, Instant>,
    missing: HashMap<RepairKey, MissingPart>,
    missing_order: VecDeque<RepairKey>,
    active_repairs: HashMap<RepairKey, usize>,
    active_repair_queries: usize,
    stats: PlumtreeStats,
}

impl State {
    pub(crate) fn new(
        config: PlumtreeConfig,
        overlay: OverlayId,
        local_source: Option<Int256>,
        local_peer: Option<PeerId>,
        receive_broadcasts: bool,
    ) -> Self {
        let slots = (0..config.tree_slots).map(|_| Slot::default()).collect();
        Self {
            config,
            overlay,
            local_source,
            local_peer,
            receive_broadcasts,
            known_peers: Vec::new(),
            slots,
            peer_activity: HashMap::new(),
            broadcasts: HashMap::new(),
            delivered: HashMap::new(),
            missing: HashMap::new(),
            missing_order: VecDeque::new(),
            active_repairs: HashMap::new(),
            active_repair_queries: 0,
            stats: PlumtreeStats::default(),
        }
    }

    pub(crate) fn stats(&self) -> PlumtreeStats {
        self.stats
    }

    #[cfg(test)]
    pub(crate) fn advertise_for_test(&mut self, key: RepairKey, peer: PeerId) -> Option<Vec<u8>> {
        self.send_ihave_to(key.0, key.1, key.2, peer)
    }

    pub(crate) fn note_validation_rejection(&mut self) {
        self.stats.validation_rejections = self.stats.validation_rejections.saturating_add(1);
        protocol_stats::record_plumtree_validation_rejection();
    }

    /// Replaces the known neighbour set and seeds every tree with a rotating
    /// subset of it.
    ///
    /// Upstream only fills the eager set when a node sends a broadcast, so a
    /// passive receiver starts with no eager peers and repairs every first
    /// payload through `IHave`.  The SDK seeds eager slots directly: relays
    /// then forward immediately after a restart instead of relying on the
    /// repair timeout, and the first payload from a seeded peer is accepted as
    /// a regular push rather than as an unsolicited frame.
    pub(crate) fn set_peers(&mut self, peers: &[PeerId]) {
        self.known_peers.clear();
        self.known_peers.extend(
            peers
                .iter()
                .copied()
                .filter(|peer| Some(*peer) != self.local_peer),
        );
        self.seed_all_slots();
    }

    pub(crate) fn start_simple(
        &mut self,
        id: PlumtreeBroadcastId,
        flags: i32,
        timestamp: f64,
        payload: Vec<u8>,
        part: PartInput,
    ) -> Result<Vec<Action>, OverlayError> {
        let key = id.as_bytes();
        if key == [0; 32] {
            return Err(OverlayError::InvalidPlumtreePayload(
                "empty Plumtree broadcast id".to_owned(),
            ));
        }
        if self.local_source.is_none() {
            return Err(OverlayError::MissingLocalKey);
        }
        if payload.is_empty() || payload.len() > MAX_PLUMTREE_BROADCAST_SIZE {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree simple payload size".to_owned(),
            ));
        }
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(
            &Int256(key),
            part.part_index.cast_signed(),
            part.tree_index.cast_signed(),
            &self.config,
        )?;
        if let Some(existing) = self.broadcasts.get(&key) {
            let same = matches!(
                existing,
                Broadcast::Simple(state)
                    if state.flags == flags
                        && state.part.data_hash == part.data_hash
                        && state.part.source_key == part.source_key
            );
            if same {
                return Ok(Vec::new());
            }
            return Err(OverlayError::PlumtreeBroadcastConflict);
        }
        if self.delivered.contains_key(&key) {
            return Ok(Vec::new());
        }

        self.seed_eager(part.tree_index);
        let simple = SimpleBroadcast {
            flags,
            payload,
            first_seen_at: Instant::now(),
            part: PartState::from(part),
        };
        let tree = simple.part.tree_index;
        self.broadcasts.insert(key, Broadcast::Simple(simple));
        let mut actions = Vec::new();
        self.forward_payload(key, 0, tree, None, &mut actions);
        self.delivered.insert(key, Instant::now());
        self.stats.completed_broadcasts = self.stats.completed_broadcasts.saturating_add(1);
        protocol_stats::record_plumtree_broadcast_completed();
        Ok(actions)
    }

    pub(crate) fn start_fec(&mut self, start: FecStart) -> Result<Vec<Action>, OverlayError> {
        let FecStart {
            id,
            flags,
            full_data_hash,
            full_data_size,
            part_size,
            source_count,
            parts,
        } = start;
        let key = id.as_bytes();
        if self.local_source.is_none() {
            return Err(OverlayError::MissingLocalKey);
        }
        if full_data_size == 0
            || full_data_size as usize > MAX_PLUMTREE_BROADCAST_SIZE
            || part_size == 0
            || parts.len() != self.config.parts as usize
        {
            return Err(OverlayError::InvalidPlumtreePayload(
                "invalid Plumtree FEC fields".to_owned(),
            ));
        }
        if let Some(existing) = self.broadcasts.get(&key) {
            let same = matches!(
                existing,
                Broadcast::Fec(state)
                    if state.flags == flags
                        && state.full_data_hash == full_data_hash
                        && state.full_data_size == full_data_size
                        && state.part_size == part_size
            );
            if same {
                return Ok(Vec::new());
            }
            return Err(OverlayError::PlumtreeBroadcastConflict);
        }
        if self.delivered.contains_key(&key) {
            return Ok(Vec::new());
        }

        for tree in PLUMTREE_FEC_TREE_OFFSET..=self.config.parts {
            self.seed_eager(tree);
        }
        let part_indices = parts.iter().map(|part| part.part_index).collect::<Vec<_>>();
        let mut state = FecBroadcast {
            flags,
            full_data_hash,
            full_data_size,
            part_size,
            source_count,
            first_seen_at: Instant::now(),
            delivered: true,
            decode_failed: false,
            decoder: None,
            decoder_parts: HashSet::new(),
            parts: HashMap::new(),
        };
        for part in parts {
            state.parts.insert(part.part_index, PartState::from(part));
        }
        self.broadcasts.insert(key, Broadcast::Fec(state));
        self.delivered.insert(key, Instant::now());
        let mut actions = Vec::new();
        for part_index in part_indices {
            let tree = part_index + PLUMTREE_FEC_TREE_OFFSET;
            self.forward_payload(key, part_index, tree, None, &mut actions);
        }
        self.stats.completed_broadcasts = self.stats.completed_broadcasts.saturating_add(1);
        protocol_stats::record_plumtree_broadcast_completed();
        Ok(actions)
    }

    pub(crate) fn process(
        &mut self,
        from: PeerId,
        broadcast: OverlayBroadcast,
        origin: PayloadOrigin,
    ) -> Result<ProcessOutcome, OverlayError> {
        if from == PeerId::from_bytes([0; 32]) {
            return Err(OverlayError::InvalidPlumtreePayload(
                "missing Plumtree immediate sender".to_owned(),
            ));
        }
        if Some(from) == self.local_peer {
            return Err(OverlayError::InvalidPlumtreePayload(
                "Plumtree frame originated from the local session".to_owned(),
            ));
        }
        match broadcast {
            broadcast @ OverlayBroadcast::PlumtreeSimple { .. } => {
                self.process_simple(from, SimpleMessage::try_from(broadcast)?, origin)
            }
            broadcast @ OverlayBroadcast::PlumtreeFec { .. } => {
                self.process_fec(from, FecMessage::try_from(broadcast)?, origin)
            }
            broadcast @ OverlayBroadcast::PlumtreeIHave { .. } => {
                self.process_ihave(from, IHaveMessage::try_from(broadcast)?)
            }
            OverlayBroadcast::PlumtreePrune {
                broadcast_id,
                timestamp,
                part_index,
                tree_index,
            } => self.process_prune(
                from,
                &broadcast_id,
                timestamp.to_f64(),
                part_index,
                tree_index,
            ),
            OverlayBroadcast::PlumtreeUseful {
                broadcast_id,
                timestamp,
                part_index,
                tree_index,
            } => self.process_useful(
                from,
                &broadcast_id,
                timestamp.to_f64(),
                part_index,
                tree_index,
            ),
            OverlayBroadcast::PlumtreeStatsPush { record } => {
                self.process_stats_push(&record);
                Ok(ProcessOutcome::default())
            }
            OverlayBroadcast::BroadcastNotFound
            | OverlayBroadcast::Unicast { .. }
            | OverlayBroadcast::Broadcast { .. } => Ok(ProcessOutcome::default()),
        }
    }

    pub(crate) fn process_repair_response(
        &mut self,
        from: PeerId,
        key: RepairKey,
        data: &[u8],
    ) -> Result<ProcessOutcome, OverlayError> {
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(data).map_err(|error| {
            OverlayError::InvalidPlumtreePayload(format!(
                "invalid Plumtree repair response: {error}"
            ))
        })?;
        match broadcast {
            OverlayBroadcast::PlumtreeSimple { .. } | OverlayBroadcast::PlumtreeFec { .. } => {
                self.process(from, broadcast, PayloadOrigin::RepairResponse(key))
            }
            OverlayBroadcast::BroadcastNotFound => Ok(ProcessOutcome::default()),
            _ => Err(OverlayError::InvalidPlumtreePayload(
                "unexpected Plumtree repair response".to_owned(),
            )),
        }
    }
    fn activate_repair(&mut self, key: RepairKey, peer: PeerId) {
        let _ = peer;
        self.active_repair_queries += 1;
        *self.active_repairs.entry(key).or_default() += 1;
    }

    pub(super) fn is_eager(&self, tree: u32, peer: PeerId) -> bool {
        self.slots
            .get(tree as usize)
            .is_some_and(|slot| slot.eager.contains(&peer))
    }

    fn duplicate_payload(
        &mut self,
        from: PeerId,
        tree: u32,
        key: BroadcastKey,
        part_index: u32,
        control_tree: u32,
    ) -> ProcessOutcome {
        self.remove_eager(tree, from);
        ProcessOutcome {
            payload: None,
            actions: vec![self.prune_action(from, key, part_index, control_tree)],
            repairs: Vec::new(),
        }
    }

    fn part(&self, key: BroadcastKey, part_index: u32, tree: u32) -> Option<&PartState> {
        match self.broadcasts.get(&key)? {
            Broadcast::Simple(state)
                if state.part.part_index == part_index && state.part.tree_index == tree =>
            {
                Some(&state.part)
            }
            Broadcast::Simple(_) => None,
            Broadcast::Fec(state) => state
                .parts
                .get(&part_index)
                .filter(|part| part.tree_index == tree),
        }
    }

    fn part_mut(
        &mut self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
    ) -> Option<&mut PartState> {
        match self.broadcasts.get_mut(&key)? {
            Broadcast::Simple(state)
                if state.part.part_index == part_index && state.part.tree_index == tree =>
            {
                Some(&mut state.part)
            }
            Broadcast::Simple(_) => None,
            Broadcast::Fec(state) => state
                .parts
                .get_mut(&part_index)
                .filter(|part| part.tree_index == tree),
        }
    }

    fn remove_missing(&mut self, key: &RepairKey) {
        self.missing.remove(key);
        self.missing_order.retain(|entry| entry != key);
    }

    fn evict_missing_parts(&mut self) {
        while self.missing.len() >= MAX_PENDING_REPAIR_PARTS {
            let Some(oldest) = self.missing_order.pop_front() else {
                break;
            };
            self.missing.remove(&oldest);
        }
    }

    fn note_control_received(&mut self) {
        self.stats.control_received = self.stats.control_received.saturating_add(1);
        protocol_stats::record_plumtree_control_received();
    }
}

impl Broadcast {
    fn part_by_index(&self) -> Option<&HashMap<u32, PartState>> {
        match self {
            Self::Simple(_) => None,
            Self::Fec(state) => Some(&state.parts),
        }
    }

    fn first_seen_at(&self) -> Instant {
        match self {
            Self::Simple(state) => state.first_seen_at,
            Self::Fec(state) => state.first_seen_at,
        }
    }

    fn is_delivered(&self) -> bool {
        match self {
            Self::Simple(_) => true,
            Self::Fec(state) => state.delivered,
        }
    }
}

impl From<PartInput> for PartState {
    fn from(part: PartInput) -> Self {
        Self {
            part_index: part.part_index,
            tree_index: part.tree_index,
            timestamp: part.timestamp,
            data_size: part.data_size,
            data_hash: part.data_hash,
            source_key: part.source_key,
            certificate: part.certificate,
            signature: part.signature,
            data: part.data,
            full_sends: 0,
            advertised_to: HashSet::new(),
            full_sent_to: HashSet::new(),
        }
    }
}
