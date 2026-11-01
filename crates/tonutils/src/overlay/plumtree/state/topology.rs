//! Plumtree eager topology, control fan-out, and garbage collection.

use std::{collections::HashSet, time::Instant};

use crate::{
    overlay::{PeerId, protocol_stats, tl::OverlayBroadcast},
    tl::TlInt256 as Int256,
};

use super::super::wire;
use super::{
    Action, BROADCAST_LIFETIME, Broadcast, BroadcastKey, EAGER_PEER_INACTIVITY_TTL,
    EAGER_PEER_MAX_SENT_WITHOUT_ACTIVITY, PENDING_FEEDBACK_TTL, Slot, State,
};

impl State {
    pub(crate) fn tick(&mut self, now: Instant) {
        for slot in &mut self.slots {
            slot.pending_feedback.retain(|_, sent_at| {
                now.saturating_duration_since(*sent_at) < PENDING_FEEDBACK_TTL
            });
        }
        let inactive = self
            .peer_activity
            .iter()
            .filter_map(|(peer, activity)| {
                (now.saturating_duration_since(activity.last_active_at)
                    >= EAGER_PEER_INACTIVITY_TTL
                    && activity.sent_since_active > EAGER_PEER_MAX_SENT_WITHOUT_ACTIVITY)
                    .then_some(*peer)
            })
            .collect::<Vec<_>>();
        for peer in inactive {
            for tree in 0..self.slots.len() {
                self.remove_eager(tree as u32, peer);
            }
        }
        self.gc(now);
    }

    fn gc(&mut self, now: Instant) {
        self.delivered.retain(|_, first_seen| {
            now.saturating_duration_since(*first_seen) < BROADCAST_LIFETIME
        });
        let expired = self
            .broadcasts
            .iter()
            .filter_map(|(id, broadcast)| {
                (now.saturating_duration_since(broadcast.first_seen_at()) >= BROADCAST_LIFETIME)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in expired {
            if let Some(broadcast) = self.broadcasts.remove(&id)
                && !broadcast.is_delivered()
            {
                self.delivered.insert(id, now);
            }
        }
        let expired_missing = self
            .missing
            .iter()
            .filter_map(|(key, missing)| {
                (now.saturating_duration_since(missing.repair_at)
                    >= BROADCAST_LIFETIME.saturating_add(self.config.repair_timeout))
                .then_some(*key)
            })
            .collect::<Vec<_>>();
        for key in expired_missing {
            self.missing.remove(&key);
            self.missing_order.retain(|entry| entry != &key);
        }
    }

    pub(super) fn seed_all_slots(&mut self) {
        let limit = self.local_eager_limit() as usize;
        if limit == 0 || self.known_peers.is_empty() {
            return;
        }
        let peers = self.known_peers.clone();
        for tree in 0..self.slots.len() {
            let start = tree % peers.len();
            for offset in 0..peers.len() {
                let peer = peers[(start + offset) % peers.len()];
                if self.slots[tree].eager.len() >= limit {
                    break;
                }
                self.slots[tree].eager.insert(peer);
                self.peer_activity.entry(peer).or_default().last_active_at = Instant::now();
            }
        }
    }

    pub(super) fn seed_eager(&mut self, tree: u32) {
        let limit = self.local_eager_limit() as usize;
        if limit == 0 {
            return;
        }
        let local = self.local_peer;
        let peers = self.known_peers.clone();
        let Some(slot) = self.slots.get_mut(tree as usize) else {
            return;
        };
        for peer in peers {
            if Some(peer) == local || slot.eager.contains(&peer) {
                continue;
            }
            if slot.eager.len() >= limit {
                break;
            }
            slot.eager.insert(peer);
            self.peer_activity.entry(peer).or_default().last_active_at = Instant::now();
        }
    }

    pub(super) fn forward_payload(
        &mut self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
        from: Option<PeerId>,
        actions: &mut Vec<Action>,
    ) {
        self.remove_inactive_eager_peers(tree);
        self.trim_eager_to_capacity(tree);
        let eager = self
            .slots
            .get(tree as usize)
            .map(|slot| slot.eager.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut full_sent = HashSet::new();
        for peer in eager {
            if Some(peer) == from || Some(peer) == self.local_peer {
                continue;
            }
            if let Some(wire) = self.send_payload_to(key, part_index, tree, peer) {
                full_sent.insert(peer);
                actions.push(Action::Send { peer, wire });
            }
        }
        let active = self.active_neighbours(key, part_index, tree, from);
        for peer in active {
            if full_sent.contains(&peer) || self.slots[tree as usize].eager.contains(&peer) {
                continue;
            }
            if let Some(wire) = self.send_ihave_to(key, part_index, tree, peer) {
                actions.push(Action::Send { peer, wire });
            }
        }
    }

    fn active_neighbours(
        &self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
        from: Option<PeerId>,
    ) -> Vec<PeerId> {
        let mut active = self
            .peer_activity
            .iter()
            .filter_map(|(peer, activity)| {
                (Some(*peer) != from
                    && Some(*peer) != self.local_peer
                    && !self.slots[tree as usize].eager.contains(peer)
                    && !self.part(key, part_index, tree).is_some_and(|part| {
                        part.full_sent_to.contains(peer) || part.advertised_to.contains(peer)
                    }))
                .then_some((*peer, activity.last_active_at))
            })
            .collect::<Vec<_>>();
        for peer in &self.known_peers {
            if Some(*peer) != from
                && Some(*peer) != self.local_peer
                && !active.iter().any(|(known, _)| known == peer)
            {
                active.push((*peer, Instant::now()));
            }
        }
        active.sort_by_key(|(_, last_active_at)| std::cmp::Reverse(*last_active_at));
        active
            .into_iter()
            .take(self.config.active_neighbours as usize)
            .map(|(peer, _)| peer)
            .collect()
    }

    pub(super) fn send_payload_to(
        &mut self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
        peer: PeerId,
    ) -> Option<Vec<u8>> {
        let slot = self.slots.get(tree as usize)?;
        if !self.can_reserve_eager_feedback(slot, peer) {
            return None;
        }
        let part = self.part(key, part_index, tree)?;
        if part.full_sends >= self.local_eager_limit() || part.full_sent_to.contains(&peer) {
            return None;
        }
        let wire = self.part_wire(key, part_index, tree)?;
        self.register_full_send(tree, key, part_index, peer);
        Some(wire)
    }

    pub(super) fn send_ihave_to(
        &mut self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
        peer: PeerId,
    ) -> Option<Vec<u8>> {
        let slot = self.slots.get(tree as usize)?;
        if !self.can_reserve_eager_feedback(slot, peer) {
            return None;
        }
        let part = self.part(key, part_index, tree)?;
        if part.full_sends >= self.local_eager_limit()
            || part.full_sent_to.contains(&peer)
            || part.advertised_to.contains(&peer)
            || part.data_size == 0
        {
            return None;
        }
        let wire = tl_proto::serialize(OverlayBroadcast::PlumtreeIHave {
            broadcast_id: Int256(key),
            timestamp: wire::unix_seconds().into(),
            part_index: part.part_index.cast_signed(),
            tree_index: part.tree_index.cast_signed(),
            src: part.source_key.clone(),
            certificate: part.certificate.clone(),
            payload_timestamp: part.timestamp.into(),
            data_size: part.data_size.cast_signed(),
            data_hash: part.data_hash.clone(),
            signature: part.signature.clone(),
        });
        let part = self.part_mut(key, part_index, tree)?;
        part.advertised_to.insert(peer);
        self.stats.control_sent = self.stats.control_sent.saturating_add(1);
        protocol_stats::record_plumtree_control_sent();
        Some(wire)
    }

    pub(super) fn part_wire(
        &self,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
    ) -> Option<Vec<u8>> {
        match self.broadcasts.get(&key)? {
            Broadcast::Simple(state) => {
                let part = &state.part;
                Some(tl_proto::serialize(OverlayBroadcast::PlumtreeSimple {
                    flags: state.flags,
                    timestamp: part.timestamp.into(),
                    src: part.source_key.clone(),
                    certificate: part.certificate.clone(),
                    broadcast_id: Int256(key),
                    tree_index: part.tree_index.cast_signed(),
                    data: state.payload.clone(),
                    signature: part.signature.clone(),
                }))
            }
            Broadcast::Fec(state) => {
                let part = state
                    .parts
                    .get(&part_index)
                    .filter(|part| part.tree_index == tree)?;
                Some(tl_proto::serialize(OverlayBroadcast::PlumtreeFec {
                    flags: state.flags,
                    timestamp: part.timestamp.into(),
                    src: part.source_key.clone(),
                    certificate: part.certificate.clone(),
                    full_data_hash: state.full_data_hash.clone(),
                    full_data_size: state.full_data_size.cast_signed(),
                    part_index: part.part_index.cast_signed(),
                    tree_index: part.tree_index.cast_signed(),
                    data: part.data.clone(),
                    signature: part.signature.clone(),
                }))
            }
        }
    }

    pub(super) fn prune_action(
        &mut self,
        peer: PeerId,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
    ) -> Action {
        self.stats.control_sent = self.stats.control_sent.saturating_add(1);
        protocol_stats::record_plumtree_control_sent();
        Action::Send {
            peer,
            wire: tl_proto::serialize(OverlayBroadcast::PlumtreePrune {
                broadcast_id: Int256(key),
                timestamp: wire::unix_seconds().into(),
                part_index: part_index.cast_signed(),
                tree_index: tree.cast_signed(),
            }),
        }
    }

    pub(super) fn useful_action(
        &mut self,
        peer: PeerId,
        key: BroadcastKey,
        part_index: u32,
        tree: u32,
    ) -> Action {
        self.stats.control_sent = self.stats.control_sent.saturating_add(1);
        protocol_stats::record_plumtree_control_sent();
        Action::Send {
            peer,
            wire: tl_proto::serialize(OverlayBroadcast::PlumtreeUseful {
                broadcast_id: Int256(key),
                timestamp: wire::unix_seconds().into(),
                part_index: part_index.cast_signed(),
                tree_index: tree.cast_signed(),
            }),
        }
    }

    pub(super) fn register_full_send(
        &mut self,
        tree: u32,
        key: BroadcastKey,
        part_index: u32,
        peer: PeerId,
    ) {
        if let Some(slot) = self.slots.get_mut(tree as usize) {
            if slot.eager.contains(&peer) {
                slot.pending_feedback.remove(&peer);
                if let Some(activity) = self.peer_activity.get_mut(&peer) {
                    activity.sent_since_active = activity.sent_since_active.saturating_add(1);
                }
            } else {
                slot.pending_feedback.insert(peer, Instant::now());
            }
        }
        if let Some(part) = self.part_mut(key, part_index, tree) {
            part.full_sent_to.insert(peer);
            part.full_sends = part.full_sends.saturating_add(1);
        }
        self.stats.parts_sent = self.stats.parts_sent.saturating_add(1);
        protocol_stats::record_plumtree_part_sent();
    }

    pub(super) fn can_reserve_eager_feedback(&self, slot: &Slot, peer: PeerId) -> bool {
        slot.eager.contains(&peer)
            || slot.pending_feedback.contains_key(&peer)
            || slot.eager.len() + slot.pending_feedback.len() < self.local_eager_limit() as usize
    }

    pub(super) fn promote_eager(&mut self, tree: u32, peer: PeerId, force: bool) {
        if peer == PeerId::from_bytes([0; 32]) || Some(peer) == self.local_peer {
            return;
        }
        let limit = self.local_eager_limit();
        if limit == 0 {
            return;
        }
        let Some(slot) = self.slots.get_mut(tree as usize) else {
            return;
        };
        slot.pending_feedback.remove(&peer);
        if slot.eager.contains(&peer) {
            return;
        }
        if slot.eager.len() >= limit as usize {
            if !force {
                return;
            }
            if let Some(removed) = slot.eager.iter().next().copied() {
                slot.pending_feedback.remove(&removed);
                slot.eager.remove(&removed);
            }
        }
        slot.eager.insert(peer);
        self.peer_activity.entry(peer).or_default().last_active_at = Instant::now();
    }

    pub(super) fn remove_eager(&mut self, tree: u32, peer: PeerId) {
        if let Some(slot) = self.slots.get_mut(tree as usize) {
            slot.eager.remove(&peer);
            slot.pending_feedback.remove(&peer);
        }
    }

    fn remove_inactive_eager_peers(&mut self, tree: u32) {
        let now = Instant::now();
        let inactive = self
            .slots
            .get(tree as usize)
            .map(|slot| {
                slot.eager
                    .iter()
                    .copied()
                    .filter(|peer| {
                        self.peer_activity.get(peer).is_some_and(|activity| {
                            now.saturating_duration_since(activity.last_active_at)
                                >= EAGER_PEER_INACTIVITY_TTL
                                && activity.sent_since_active > EAGER_PEER_MAX_SENT_WITHOUT_ACTIVITY
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for peer in inactive {
            self.remove_eager(tree, peer);
        }
    }

    fn trim_eager_to_capacity(&mut self, tree: u32) {
        let limit = self.local_eager_limit() as usize;
        if let Some(slot) = self.slots.get_mut(tree as usize) {
            while slot.eager.len() > limit {
                if let Some(removed) = slot.eager.iter().next().copied() {
                    slot.eager.remove(&removed);
                    slot.pending_feedback.remove(&removed);
                } else {
                    break;
                }
            }
        }
    }

    pub(super) fn note_eager_active(&mut self, peer: PeerId) {
        if let Some(activity) = self.peer_activity.get_mut(&peer) {
            activity.last_active_at = Instant::now();
            activity.sent_since_active = 0;
        }
    }

    pub(super) fn local_eager_limit(&self) -> u32 {
        if self.receive_broadcasts {
            self.config.eager_limit
        } else {
            self.config.validator_eager_limit
        }
    }
}
