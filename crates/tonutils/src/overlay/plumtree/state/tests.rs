use std::time::{Duration, Instant};

use crate::{
    adnl::AdnlKeyPair,
    overlay::{OverlayId, tl::OverlayBroadcast},
    tl::TlInt256 as Int256,
};

use super::{
    Action, BROADCAST_LIFETIME, Broadcast, MAX_ACTIVE_REPAIR_QUERIES, MAX_PENDING_REPAIR_PARTS,
    MissingPart, PayloadOrigin, PlumtreeBroadcastId, PlumtreeConfig, State, wire,
};

fn config() -> PlumtreeConfig {
    PlumtreeConfig {
        k: 4,
        parts: 6,
        tree_slots: 7,
        validator_eager_limit: 2,
        eager_limit: 2,
        active_neighbours: 2,
        repair_timeout: Duration::from_millis(50),
        max_repair_targets: 2,
        enabled: true,
    }
}

fn keypair() -> AdnlKeyPair {
    AdnlKeyPair::generate(&mut rand::rngs::OsRng)
}

fn state(config: PlumtreeConfig, receive_broadcasts: bool, local_peer: u8) -> State {
    State::new(
        config,
        OverlayId::from_name(b"plumtree-state-tests"),
        Some(Int256([9; 32])),
        Some(super::PeerId::from_bytes([local_peer; 32])),
        receive_broadcasts,
    )
}

fn earlier(now: Instant, duration: Duration) -> Instant {
    now.checked_sub(duration)
        .expect("test timestamps are later than the simulated durations")
}

fn signed_simple(
    keypair: &AdnlKeyPair,
    id: &Int256,
    payload: &[u8],
    timestamp: f64,
) -> (OverlayBroadcast, Vec<u8>) {
    let (wire, _) = wire::simple_signed_part(keypair, 0, timestamp, id.clone(), payload).unwrap();
    (tl_proto::deserialize(&wire).unwrap(), wire)
}

fn ihave(
    keypair: &AdnlKeyPair,
    id: &Int256,
    payload: &[u8],
    payload_timestamp: f64,
) -> OverlayBroadcast {
    let (_, part) =
        wire::simple_signed_part(keypair, 0, payload_timestamp, id.clone(), payload).unwrap();
    OverlayBroadcast::PlumtreeIHave {
        broadcast_id: id.clone(),
        timestamp: wire::unix_seconds().into(),
        part_index: 0,
        tree_index: 0,
        src: part.source_key,
        certificate: crate::overlay::tl::OverlayCertificate::Empty,
        payload_timestamp: payload_timestamp.into(),
        data_size: part.data_size.cast_signed(),
        data_hash: part.data_hash,
        signature: part.signature,
    }
}

fn fec_ihave(
    keypair: &AdnlKeyPair,
    encoding: &wire::FecEncoding,
    payload_timestamp: f64,
    part_index: u32,
) -> OverlayBroadcast {
    let (_, part) = wire::fec_signed_part(
        keypair,
        wire::FecPartSpec {
            flags: 0,
            timestamp: payload_timestamp,
            id: encoding.broadcast_id.clone(),
            full_data_hash: encoding.full_data_hash.clone(),
            full_data_size: encoding.full_data_size.cast_signed(),
            part_index,
        },
        &encoding.symbols[part_index as usize],
    )
    .unwrap();
    OverlayBroadcast::PlumtreeIHave {
        broadcast_id: encoding.broadcast_id.clone(),
        timestamp: wire::unix_seconds().into(),
        part_index: part.part_index.cast_signed(),
        tree_index: part.tree_index.cast_signed(),
        src: part.source_key,
        certificate: crate::overlay::tl::OverlayCertificate::Empty,
        payload_timestamp: payload_timestamp.into(),
        data_size: part.data_size.cast_signed(),
        data_hash: part.data_hash,
        signature: part.signature,
    }
}

#[test]
fn originating_simple_broadcast_sends_to_eager_peers() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0xAA);
    state.set_peers(&[peer]);
    let id = Int256([0x42; 32]);
    let payload = b"plumtree payload".to_vec();
    let timestamp = wire::unix_seconds();
    let (_, part) = wire::simple_signed_part(&sender, 0, timestamp, id.clone(), &payload).unwrap();

    let actions = state
        .start_simple(
            PlumtreeBroadcastId::from(id.clone()),
            0,
            timestamp,
            payload.clone(),
            part,
        )
        .unwrap();
    assert_eq!(actions.len(), 1, "one eager peer must receive one frame");
    let Action::Send {
        peer: sent_to,
        wire,
    } = &actions[0]
    else {
        panic!("the originating frame must be a payload send");
    };
    assert_eq!(*sent_to, peer);
    let broadcast = tl_proto::deserialize::<OverlayBroadcast>(wire).unwrap();
    let OverlayBroadcast::PlumtreeSimple {
        broadcast_id,
        data,
        tree_index,
        ..
    } = broadcast
    else {
        panic!("the queued frame must be a Plumtree simple broadcast");
    };
    assert_eq!(broadcast_id, id);
    assert_eq!(data, payload);
    assert_eq!(tree_index, 0);
    assert_eq!(state.stats().parts_sent, 1);
    assert_eq!(state.stats().completed_broadcasts, 1);
}

#[test]
fn eager_push_is_delivered_once_and_duplicates_are_pruned() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0xBB);
    state.set_peers(&[sender_peer]);
    let id = Int256([0x11; 32]);
    let payload = b"eager payload";
    let timestamp = wire::unix_seconds();
    let (broadcast, _) = signed_simple(&sender, &id, payload, timestamp);

    let outcome = state
        .process(sender_peer, broadcast.clone(), PayloadOrigin::Push)
        .unwrap();
    assert_eq!(outcome.payload.as_deref(), Some(payload.as_slice()));
    assert_eq!(state.stats().parts_received, 1);
    assert_eq!(state.stats().completed_broadcasts, 1);
    assert!(state.is_eager(0, sender_peer), "the sender stays eager");

    let duplicate = state
        .process(sender_peer, broadcast, PayloadOrigin::Push)
        .unwrap();
    assert!(duplicate.payload.is_none(), "duplicates are not delivered");
    assert_eq!(state.stats().parts_received, 1);
    assert!(
        duplicate
            .actions
            .iter()
            .any(|action| matches!(action, Action::Send { peer, .. } if *peer == sender_peer)),
        "a duplicate push must be answered with a prune"
    );
    assert!(!state.is_eager(0, sender_peer));
}

#[test]
fn push_from_a_non_eager_peer_is_pruned_without_delivery() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let eager_peer = super::PeerId::from_bytes([2; 32]);
    let mut state = state(config(), true, 0xCC);
    state.set_peers(&[eager_peer]);
    let id = Int256([0x22; 32]);
    let (broadcast, _) = signed_simple(&sender, &id, b"unexpected", wire::unix_seconds());

    let outcome = state
        .process(sender_peer, broadcast, PayloadOrigin::Push)
        .unwrap();
    assert!(outcome.payload.is_none());
    assert!(
        outcome
            .actions
            .iter()
            .any(|action| matches!(action, Action::Send { peer, .. } if *peer == sender_peer)),
        "a non-eager push must be answered with a prune"
    );
    assert!(!state.is_eager(0, sender_peer));
}

#[test]
fn ihave_starts_an_immediate_repair_without_an_eager_slot() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0xDD);
    let id = Int256([0x33; 32]);
    let payload = b"repairable payload";
    let payload_timestamp = wire::unix_seconds();
    let message = ihave(&sender, &id, payload, payload_timestamp);

    let outcome = state
        .process(sender_peer, message, PayloadOrigin::Push)
        .unwrap();
    assert_eq!(outcome.repairs.len(), 1);
    assert_eq!(outcome.repairs[0].key, (id.0, 0, 0));
    assert_eq!(outcome.repairs[0].peer, sender_peer);
    assert_eq!(state.stats().control_received, 1);

    let (repair, _) = signed_simple(&sender, &id, payload, payload_timestamp);
    let repaired = state
        .process_repair_response(sender_peer, (id.0, 0, 0), &tl_proto::serialize(&repair))
        .unwrap();
    assert_eq!(repaired.payload.as_deref(), Some(payload.as_slice()));
    assert!(
        state
            .process_repair_response(sender_peer, (id.0, 0, 1), &tl_proto::serialize(&repair))
            .is_err(),
        "a repair response for another part must be rejected"
    );
}

#[test]
fn ihave_waits_for_the_repair_timeout_when_an_eager_slot_exists() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0xEE);
    state.set_peers(&[super::PeerId::from_bytes([2; 32])]);
    let id = Int256([0x44; 32]);
    let message = ihave(&sender, &id, b"delayed repair", wire::unix_seconds());

    let outcome = state
        .process(sender_peer, message, PayloadOrigin::Push)
        .unwrap();
    assert!(
        outcome.repairs.is_empty(),
        "an occupied eager slot delays the repair"
    );
    let now = Instant::now();
    assert!(state.due_repairs(now).is_empty());
    let due = state.due_repairs(now + config().repair_timeout + Duration::from_millis(1));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].key, (id.0, 0, 0));
    assert_eq!(due[0].peer, sender_peer);
}

#[test]
fn prune_removes_the_peer_that_received_the_payload() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0x11);
    state.set_peers(&[peer]);
    let id = Int256([0x55; 32]);
    let timestamp = wire::unix_seconds();
    let (_, part) =
        wire::simple_signed_part(&sender, 0, timestamp, id.clone(), b"payload").unwrap();
    state
        .start_simple(
            PlumtreeBroadcastId::from(id.clone()),
            0,
            timestamp,
            b"payload".to_vec(),
            part,
        )
        .unwrap();
    assert!(state.is_eager(0, peer));

    let prune = OverlayBroadcast::PlumtreePrune {
        broadcast_id: id,
        timestamp: wire::unix_seconds().into(),
        part_index: 0,
        tree_index: 0,
    };
    state.process(peer, prune, PayloadOrigin::Push).unwrap();
    assert!(!state.is_eager(0, peer));
}

#[test]
fn useful_promotes_a_pending_neighbour_to_eager() {
    let sender = keypair();
    let eager_peer = super::PeerId::from_bytes([1; 32]);
    let pending_peer = super::PeerId::from_bytes([2; 32]);
    let mut state = state(config(), true, 0x22);
    state.set_peers(&[eager_peer]);
    let id = Int256([0x66; 32]);
    let timestamp = wire::unix_seconds();
    let (_, part) =
        wire::simple_signed_part(&sender, 0, timestamp, id.clone(), b"payload").unwrap();
    state
        .start_simple(
            PlumtreeBroadcastId::from(id.clone()),
            0,
            timestamp,
            b"payload".to_vec(),
            part,
        )
        .unwrap();
    assert!(
        state.send_payload_to(id.0, 0, 0, pending_peer).is_some(),
        "the pending peer must receive the payload"
    );
    assert!(!state.is_eager(0, pending_peer));

    let useful = OverlayBroadcast::PlumtreeUseful {
        broadcast_id: id,
        timestamp: wire::unix_seconds().into(),
        part_index: 0,
        tree_index: 0,
    };
    state
        .process(pending_peer, useful, PayloadOrigin::Push)
        .unwrap();
    assert!(state.is_eager(0, pending_peer));
}

#[test]
fn answer_repair_serves_advertised_parts_once() {
    let sender = keypair();
    let eager_peer = super::PeerId::from_bytes([1; 32]);
    let requesting_peer = super::PeerId::from_bytes([2; 32]);
    let mut state = state(config(), true, 0x33);
    state.set_peers(&[eager_peer]);
    let id = Int256([0x77; 32]);
    let timestamp = wire::unix_seconds();
    let (_, part) =
        wire::simple_signed_part(&sender, 0, timestamp, id.clone(), b"payload").unwrap();
    state
        .start_simple(
            PlumtreeBroadcastId::from(id.clone()),
            0,
            timestamp,
            b"payload".to_vec(),
            part,
        )
        .unwrap();
    assert!(
        state.send_ihave_to(id.0, 0, 0, requesting_peer).is_some(),
        "the part must be advertised before it can be repaired"
    );

    let answer = state
        .answer_repair(requesting_peer, &id, wire::unix_seconds(), 0, 0)
        .unwrap();
    let answer = answer.expect("an advertised part must be served");
    let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&answer).unwrap();
    assert!(matches!(broadcast, OverlayBroadcast::PlumtreeSimple { .. }));
    assert!(
        state
            .answer_repair(requesting_peer, &id, wire::unix_seconds(), 0, 0)
            .unwrap()
            .is_none(),
        "a part must not be served twice to the same peer"
    );
}

#[test]
fn fec_parts_reconstruct_the_payload() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let config = config();
    let payload = (0..300u32)
        .map(|value| u8::try_from(value % 251).unwrap())
        .collect::<Vec<_>>();
    let encoding = wire::encode_fec(&payload, 0, &config).unwrap();
    let timestamp = wire::unix_seconds();
    let mut state = state(config, true, 0x44);
    state.set_peers(&[sender_peer]);
    let mut decoded = None;
    for (index, symbol) in encoding.symbols.iter().enumerate() {
        let index = u32::try_from(index).unwrap();
        let (wire, _) = wire::fec_signed_part(
            &sender,
            wire::FecPartSpec {
                flags: 0,
                timestamp,
                id: encoding.broadcast_id.clone(),
                full_data_hash: encoding.full_data_hash.clone(),
                full_data_size: i32::try_from(payload.len()).unwrap(),
                part_index: index,
            },
            symbol,
        )
        .unwrap();
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
        let outcome = state
            .process(sender_peer, broadcast, PayloadOrigin::Push)
            .unwrap();
        if outcome.payload.is_some() {
            decoded = outcome.payload;
        }
    }
    assert_eq!(decoded.as_deref(), Some(payload.as_slice()));
    assert_eq!(state.stats().parts_received, u64::from(config.parts));
    assert_eq!(state.stats().completed_broadcasts, 1);
}

#[test]
fn set_peers_rotates_the_eager_seed_across_trees() {
    let config = PlumtreeConfig {
        k: 2,
        parts: 2,
        tree_slots: 4,
        eager_limit: 1,
        ..config()
    };
    let mut state = state(config, true, 0x55);
    let peers = [
        super::PeerId::from_bytes([1; 32]),
        super::PeerId::from_bytes([2; 32]),
        super::PeerId::from_bytes([3; 32]),
    ];
    state.set_peers(&peers);
    for tree in 0..4usize {
        assert!(
            state.is_eager(tree as u32, peers[tree % peers.len()]),
            "tree {tree} must start on the rotating peer"
        );
        assert_eq!(state.slots[tree].eager.len(), 1);
    }
}

#[test]
fn tick_expires_delivered_broadcasts() {
    let mut state = state(config(), true, 0x66);
    let id = [0x88; 32];
    let now = Instant::now();
    state
        .delivered
        .insert(id, earlier(now, Duration::from_secs(21)));
    state.tick(now);
    assert!(!state.delivered.contains_key(&id));
}

#[test]
fn repeated_simple_id_with_different_data_or_flags_is_a_conflict() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0x77);
    state.set_peers(&[peer]);
    let id = Int256([0x91; 32]);
    let timestamp = wire::unix_seconds();
    let (first, _) = signed_simple(&sender, &id, b"first", timestamp);
    assert_eq!(
        state
            .process(peer, first, PayloadOrigin::Push)
            .unwrap()
            .payload
            .as_deref(),
        Some(b"first".as_slice())
    );

    let (different_data, _) = signed_simple(&sender, &id, b"second", timestamp);
    assert!(matches!(
        state.process(peer, different_data, PayloadOrigin::Push),
        Err(crate::overlay::OverlayError::PlumtreeBroadcastConflict)
    ));

    let (wire, _) = wire::simple_signed_part(&sender, 1, timestamp, id, b"first").unwrap();
    let different_flags = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
    assert!(matches!(
        state.process(peer, different_flags, PayloadOrigin::Push),
        Err(crate::overlay::OverlayError::PlumtreeBroadcastConflict)
    ));
}

#[test]
fn repeated_fec_part_with_different_bytes_is_a_conflict() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let config = config();
    let payload = vec![7u8; 300];
    let encoding = wire::encode_fec(&payload, 0, &config).unwrap();
    let timestamp = wire::unix_seconds();
    let mut state = state(config, true, 0x78);
    state.set_peers(&[peer]);

    let (wire, _) = wire::fec_signed_part(
        &sender,
        wire::FecPartSpec {
            flags: 0,
            timestamp,
            id: encoding.broadcast_id.clone(),
            full_data_hash: encoding.full_data_hash.clone(),
            full_data_size: i32::try_from(payload.len()).unwrap(),
            part_index: 0,
        },
        &encoding.symbols[0],
    )
    .unwrap();
    state
        .process(
            peer,
            tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap(),
            PayloadOrigin::Push,
        )
        .unwrap();

    let different = vec![0u8; encoding.symbols[0].len()];
    let (wire, _) = wire::fec_signed_part(
        &sender,
        wire::FecPartSpec {
            flags: 0,
            timestamp,
            id: encoding.broadcast_id,
            full_data_hash: encoding.full_data_hash,
            full_data_size: i32::try_from(payload.len()).unwrap(),
            part_index: 0,
        },
        &different,
    )
    .unwrap();
    assert!(matches!(
        state.process(
            peer,
            tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap(),
            PayloadOrigin::Push
        ),
        Err(crate::overlay::OverlayError::PlumtreeBroadcastConflict)
    ));
}

#[test]
fn do_not_receive_does_not_publish_or_relay_but_can_originate() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), false, 0x79);
    state.set_peers(&[peer]);
    let id = Int256([0x92; 32]);
    let timestamp = wire::unix_seconds();
    let (incoming, _) = signed_simple(&sender, &id, b"incoming", timestamp);

    let outcome = state.process(peer, incoming, PayloadOrigin::Push).unwrap();
    assert!(outcome.payload.is_none());
    assert_eq!(state.stats().parts_received, 0);
    assert_eq!(state.stats().completed_broadcasts, 0);
    assert_eq!(outcome.actions.len(), 1, "the push must only be pruned");
    let Action::Send {
        peer: prune_to,
        wire,
    } = &outcome.actions[0]
    else {
        panic!("the no-receive response must be a control frame");
    };
    assert_eq!(*prune_to, peer);
    assert!(matches!(
        tl_proto::deserialize::<OverlayBroadcast>(wire).unwrap(),
        OverlayBroadcast::PlumtreePrune { .. }
    ));

    let outgoing_id = Int256([0x93; 32]);
    let (_, part) =
        wire::simple_signed_part(&sender, 0, timestamp, outgoing_id.clone(), b"outgoing").unwrap();
    let actions = state
        .start_simple(
            PlumtreeBroadcastId::from(outgoing_id),
            0,
            timestamp,
            b"outgoing".to_vec(),
            part,
        )
        .unwrap();
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            Action::Send { wire, .. }
                if matches!(
                    tl_proto::deserialize::<OverlayBroadcast>(wire),
                    Ok(OverlayBroadcast::PlumtreeSimple { .. })
                )
        )
    }));
}

#[test]
fn pending_repair_quota_evicts_the_oldest_part() {
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0x7A);
    for index in 0..MAX_PENDING_REPAIR_PARTS {
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&u64::try_from(index).unwrap().to_le_bytes());
        let repair_key = (key, 0, 0);
        state.missing.insert(
            repair_key,
            MissingPart {
                targets: vec![peer],
                repair_at: Instant::now() + config().repair_timeout,
            },
        );
        state.missing_order.push_back(repair_key);
    }

    let sender = keypair();
    let id = Int256([0x94; 32]);
    let message = ihave(&sender, &id, b"new repair", wire::unix_seconds());
    state.process(peer, message, PayloadOrigin::Push).unwrap();

    assert_eq!(state.missing.len(), MAX_PENDING_REPAIR_PARTS);
    assert!(state.missing.contains_key(&(id.0, 0, 0)));
    assert!(
        !state.missing.contains_key(&([0u8; 32], 0, 0)),
        "the oldest missing part must be evicted"
    );
}

#[test]
fn active_repair_quota_saturates_and_releases_once() {
    let peer = super::PeerId::from_bytes([1; 32]);
    let first = ([1u8; 32], 0, 0);
    let mut state = state(config(), true, 0x7B);
    state.activate_repair(first, peer);
    assert_eq!(state.active_repair_queries, 1);
    state.finish_repair(first);
    assert_eq!(state.active_repair_queries, 0);
    state.finish_repair(first);
    assert_eq!(
        state.active_repair_queries, 0,
        "finishing the same repair twice must not release another slot"
    );

    for index in 0..MAX_ACTIVE_REPAIR_QUERIES {
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&u64::try_from(index).unwrap().to_le_bytes());
        state.activate_repair((key, 0, 0), peer);
    }
    assert_eq!(state.active_repair_queries, MAX_ACTIVE_REPAIR_QUERIES);

    let sender = keypair();
    let id = Int256([0x95; 32]);
    let outcome = state
        .process(
            peer,
            ihave(&sender, &id, b"over quota", wire::unix_seconds()),
            PayloadOrigin::Push,
        )
        .unwrap();
    assert!(
        outcome.repairs.is_empty(),
        "the active repair quota must cap immediate requests"
    );
}

#[test]
fn delayed_repair_recovers_a_lost_fec_part_once() {
    let sender = keypair();
    let sender_peer = super::PeerId::from_bytes([1; 32]);
    let repair_peer = super::PeerId::from_bytes([2; 32]);
    let config = config();
    let payload = (0..300u32)
        .map(|value| u8::try_from(value % 251).unwrap())
        .collect::<Vec<_>>();
    let encoding = wire::encode_fec(&payload, 0, &config).unwrap();
    let timestamp = wire::unix_seconds();
    let mut state = state(config, true, 0x7C);
    state.set_peers(&[sender_peer]);

    for part_index in [0u32, 1, 3] {
        let (wire, _) = wire::fec_signed_part(
            &sender,
            wire::FecPartSpec {
                flags: 0,
                timestamp,
                id: encoding.broadcast_id.clone(),
                full_data_hash: encoding.full_data_hash.clone(),
                full_data_size: i32::try_from(payload.len()).unwrap(),
                part_index,
            },
            &encoding.symbols[part_index as usize],
        )
        .unwrap();
        let outcome = state
            .process(
                sender_peer,
                tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap(),
                PayloadOrigin::Push,
            )
            .unwrap();
        assert!(outcome.payload.is_none());
    }

    state.remove_eager(3, sender_peer);
    let outcome = state
        .process(
            repair_peer,
            fec_ihave(&sender, &encoding, timestamp, 2),
            PayloadOrigin::Push,
        )
        .unwrap();
    assert_eq!(outcome.repairs.len(), 1);
    let key = (encoding.broadcast_id.0, 2, 3);
    assert_eq!(outcome.repairs[0].key, key);
    assert_eq!(outcome.repairs[0].peer, repair_peer);

    let (wire, _) = wire::fec_signed_part(
        &sender,
        wire::FecPartSpec {
            flags: 0,
            timestamp,
            id: encoding.broadcast_id,
            full_data_hash: encoding.full_data_hash,
            full_data_size: i32::try_from(payload.len()).unwrap(),
            part_index: 2,
        },
        &encoding.symbols[2],
    )
    .unwrap();
    let repaired = state
        .process_repair_response(repair_peer, key, &wire)
        .unwrap();
    assert_eq!(repaired.payload.as_deref(), Some(payload.as_slice()));
    let duplicate = state
        .process_repair_response(repair_peer, key, &wire)
        .unwrap();
    assert!(duplicate.payload.is_none());
    assert_eq!(state.stats().completed_broadcasts, 1);
    assert_eq!(state.stats().repair_requests, 0);
}

#[test]
fn invalid_sizes_timestamps_and_part_bounds_are_rejected() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let mut state = state(config(), true, 0x7D);
    state.set_peers(&[peer]);
    let id = Int256([0x96; 32]);
    let timestamp = wire::unix_seconds();
    let (wire, _) =
        wire::simple_signed_part(&sender, 0, timestamp, id.clone(), b"payload").unwrap();
    let base = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();

    let mut old_timestamp = base.clone();
    if let OverlayBroadcast::PlumtreeSimple {
        timestamp: message_timestamp,
        ..
    } = &mut old_timestamp
    {
        *message_timestamp = (wire::unix_seconds() - 21.0).into();
    }
    assert!(
        state
            .process(peer, old_timestamp, PayloadOrigin::Push)
            .is_err()
    );

    let mut empty_payload = base.clone();
    if let OverlayBroadcast::PlumtreeSimple { data, .. } = &mut empty_payload {
        data.clear();
    }
    assert!(
        state
            .process(peer, empty_payload, PayloadOrigin::Push)
            .is_err()
    );

    let mut bad_part = base.clone();
    if let OverlayBroadcast::PlumtreeSimple { tree_index, .. } = &mut bad_part {
        *tree_index = 1;
    }
    assert!(state.process(peer, bad_part, PayloadOrigin::Push).is_err());

    let mut bad_tree = base;
    if let OverlayBroadcast::PlumtreeSimple { tree_index, .. } = &mut bad_tree {
        *tree_index = 7;
    }
    assert!(state.process(peer, bad_tree, PayloadOrigin::Push).is_err());

    let encoding = wire::encode_fec(b"fec payload", 0, &config()).unwrap();
    let (wire, _) = wire::fec_signed_part(
        &sender,
        wire::FecPartSpec {
            flags: 0,
            timestamp,
            id: encoding.broadcast_id,
            full_data_hash: encoding.full_data_hash,
            full_data_size: 11,
            part_index: 0,
        },
        &encoding.symbols[0],
    )
    .unwrap();
    let mut bad_size = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
    if let OverlayBroadcast::PlumtreeFec { full_data_size, .. } = &mut bad_size {
        *full_data_size = 0;
    }
    assert!(state.process(peer, bad_size, PayloadOrigin::Push).is_err());
}

#[test]
fn gc_expires_broadcasts_and_pending_repairs_after_twenty_seconds() {
    let sender = keypair();
    let peer = super::PeerId::from_bytes([1; 32]);
    let config = config();
    let payload = vec![3u8; 300];
    let encoding = wire::encode_fec(&payload, 0, &config).unwrap();
    let timestamp = wire::unix_seconds();
    let mut state = state(config, true, 0x7E);
    state.set_peers(&[peer]);
    let (wire, _) = wire::fec_signed_part(
        &sender,
        wire::FecPartSpec {
            flags: 0,
            timestamp,
            id: encoding.broadcast_id.clone(),
            full_data_hash: encoding.full_data_hash,
            full_data_size: i32::try_from(payload.len()).unwrap(),
            part_index: 0,
        },
        &encoding.symbols[0],
    )
    .unwrap();
    state
        .process(
            peer,
            tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap(),
            PayloadOrigin::Push,
        )
        .unwrap();

    let now = Instant::now();
    if let Some(Broadcast::Fec(broadcast)) = state.broadcasts.get_mut(&encoding.broadcast_id.0) {
        broadcast.first_seen_at = earlier(earlier(now, BROADCAST_LIFETIME), Duration::from_secs(1));
    }
    let missing_key = ([0x97; 32], 0, 0);
    state.missing.insert(
        missing_key,
        MissingPart {
            targets: vec![peer],
            repair_at: earlier(
                earlier(earlier(now, BROADCAST_LIFETIME), config.repair_timeout),
                Duration::from_secs(1),
            ),
        },
    );
    state.missing_order.push_back(missing_key);

    state.tick(now);
    assert!(!state.broadcasts.contains_key(&encoding.broadcast_id.0));
    assert!(state.delivered.contains_key(&encoding.broadcast_id.0));
    assert!(!state.missing.contains_key(&missing_key));
}
