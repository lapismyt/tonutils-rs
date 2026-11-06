//! Plumtree repair queries and pending-repair bookkeeping.

use std::time::Instant;

use crate::{
    overlay::{OverlayError, PeerId, protocol_stats, tl::OverlayBroadcast},
    tl::TlInt256 as Int256,
};

use super::super::wire;
use super::{MAX_ACTIVE_REPAIR_QUERIES, RepairKey, RepairRequest, State};

impl State {
    pub(crate) fn answer_repair(
        &mut self,
        from: PeerId,
        broadcast_id: &Int256,
        timestamp: f64,
        part_index: i32,
        tree_index: i32,
    ) -> Result<Option<Vec<u8>>, OverlayError> {
        wire::check_timestamp(timestamp, wire::unix_seconds())?;
        wire::check_control_fields(broadcast_id, part_index, tree_index, &self.config)?;
        let part_index = u32::try_from(part_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree part index".to_owned())
        })?;
        let tree = u32::try_from(tree_index).map_err(|_| {
            OverlayError::InvalidPlumtreePayload("negative Plumtree tree index".to_owned())
        })?;
        self.stats.repair_requests = self.stats.repair_requests.saturating_add(1);
        protocol_stats::record_plumtree_repair_request();
        if !self.broadcasts.contains_key(&broadcast_id.0) {
            return if self.delivered.contains_key(&broadcast_id.0) {
                Ok(Some(tl_proto::serialize(
                    OverlayBroadcast::BroadcastNotFound,
                )))
            } else {
                Ok(None)
            };
        }
        if self
            .part(broadcast_id.0, part_index, tree)
            .is_none_or(|part| {
                !part.advertised_to.contains(&from) || part.full_sent_to.contains(&from)
            })
        {
            return Ok(None);
        }
        let Some(slot) = self.slots.get(tree as usize) else {
            return Ok(None);
        };
        if !self.can_reserve_eager_feedback(slot, from) {
            return Ok(Some(tl_proto::serialize(
                OverlayBroadcast::BroadcastNotFound,
            )));
        }
        let Some(part) = self.part(broadcast_id.0, part_index, tree) else {
            return Ok(None);
        };
        if part.full_sends >= self.local_eager_limit() {
            return Ok(Some(tl_proto::serialize(
                OverlayBroadcast::BroadcastNotFound,
            )));
        }
        let wire = self.part_wire(broadcast_id.0, part_index, tree);
        self.register_full_send(tree, broadcast_id.0, part_index, from);
        Ok(wire)
    }

    pub(crate) fn due_repairs(&mut self, now: Instant) -> Vec<RepairRequest> {
        let due = self
            .missing
            .iter()
            .filter_map(|(key, missing)| (missing.repair_at <= now).then_some(*key))
            .collect::<Vec<_>>();
        let mut requests = Vec::new();
        for key in due {
            let Some(missing) = self.missing.remove(&key) else {
                continue;
            };
            self.missing_order.retain(|entry| entry != &key);
            for peer in missing.targets {
                if self.active_repair_queries >= MAX_ACTIVE_REPAIR_QUERIES {
                    break;
                }
                self.activate_repair(key, peer);
                requests.push(RepairRequest { key, peer });
            }
        }
        requests
    }

    pub(crate) fn finish_repair(&mut self, key: RepairKey) {
        let Some(count) = self.active_repairs.get_mut(&key) else {
            return;
        };
        *count = count.saturating_sub(1);
        self.active_repair_queries = self.active_repair_queries.saturating_sub(1);
        if *count == 0 {
            self.active_repairs.remove(&key);
        }
    }
}
