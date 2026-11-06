//! Shared Plumtree broadcast state for UDP and QUIC overlay sessions.

use std::{
    fmt,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
    time::{Duration, Instant},
};

use tokio::sync::mpsc;

use crate::{
    adnl::{AdnlKeyPair, tl::AdnlTlPublicKey as TlPublicKey},
    overlay::{OverlayError, OverlayId, PeerId, tl::OverlayBroadcast},
    tl::TlInt256 as Int256,
};

#[cfg(test)]
mod integration_tests;
mod state;
mod task;
mod wire;

pub(crate) use state::{Action, PayloadOrigin, RepairKey, RepairRequest};
pub(crate) use task::spawn_plumtree_tasks;

/// Upstream-compatible Plumtree tuning parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlumtreeConfig {
    /// Source symbols in one FEC broadcast.
    pub k: u32,
    /// Maximum number of FEC symbols generated for one broadcast.
    pub parts: u32,
    /// Number of independent Plumtree trees.
    pub tree_slots: u32,
    /// Eager fan-out for the original sender.
    pub validator_eager_limit: u32,
    /// Eager fan-out for relay nodes.
    pub eager_limit: u32,
    /// Number of non-eager neighbours advertised to with `IHave`.
    pub active_neighbours: u32,
    /// Delay before repair queries are sent.
    pub repair_timeout: Duration,
    /// Maximum number of repair targets for one missing part.
    pub max_repair_targets: u32,
    /// Whether this node runs the Plumtree state machine.
    pub enabled: bool,
}

impl Default for PlumtreeConfig {
    fn default() -> Self {
        Self {
            k: 30,
            parts: 45,
            tree_slots: 46,
            validator_eager_limit: 1,
            eager_limit: 4,
            active_neighbours: 5,
            repair_timeout: Duration::from_millis(200),
            max_repair_targets: 5,
            enabled: true,
        }
    }
}

impl PlumtreeConfig {
    /// Checks the invariants upstream requires before constructing a
    /// Plumtree broadcaster.
    pub fn validate(&self) -> Result<(), OverlayError> {
        if !self.enabled {
            return Ok(());
        }
        if self.k == 0
            || self.parts == 0
            || self.tree_slots <= self.parts
            || self.eager_limit == 0
            || self.max_repair_targets == 0
        {
            return Err(OverlayError::Configuration(
                "invalid Plumtree configuration".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Identity of one Plumtree simple broadcast.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PlumtreeBroadcastId([u8; 32]);

impl PlumtreeBroadcastId {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub const fn as_int256(self) -> Int256 {
        Int256(self.0)
    }
}

impl From<[u8; 32]> for PlumtreeBroadcastId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<Int256> for PlumtreeBroadcastId {
    fn from(value: Int256) -> Self {
        Self(value.0)
    }
}

impl fmt::Display for PlumtreeBroadcastId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Per-node Plumtree counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlumtreeStats {
    /// Payload parts accepted into the shared receive state.
    pub parts_received: u64,
    /// Payload parts sent to neighbours.
    pub parts_sent: u64,
    /// `IHave`, `Prune`, `Useful`, and statistics control frames received.
    pub control_received: u64,
    /// Plumtree control frames sent to neighbours.
    pub control_sent: u64,
    /// Broadcasts delivered exactly once to the application.
    pub completed_broadcasts: u64,
    /// Repair queries received or issued.
    pub repair_requests: u64,
    /// Frames rejected by timestamp, size, signature, certificate, or bounds.
    pub validation_rejections: u64,
}

#[derive(Debug)]
pub(crate) struct PartInput {
    pub part_index: u32,
    pub tree_index: u32,
    pub timestamp: f64,
    pub data_size: u32,
    pub data_hash: Int256,
    pub source_key: TlPublicKey,
    pub certificate: crate::overlay::tl::OverlayCertificate,
    pub signature: Vec<u8>,
    pub data: Vec<u8>,
}

struct HandleInner {
    config: StdRwLock<PlumtreeConfig>,
    overlay: StdRwLock<Option<OverlayId>>,
    state: tokio::sync::Mutex<state::State>,
    keypair: StdRwLock<Option<AdnlKeyPair>>,
    local_peer: StdRwLock<Option<PeerId>>,
    peers: StdRwLock<Vec<PeerId>>,
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: StdMutex<Option<mpsc::UnboundedReceiver<Action>>>,
    stats: StdMutex<PlumtreeStats>,
}

/// A cloneable handle onto one overlay's shared Plumtree state.
#[derive(Clone)]
pub struct PlumtreeHandle {
    inner: Arc<HandleInner>,
}

impl PlumtreeHandle {
    pub(crate) fn new(config: PlumtreeConfig) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let overlay = OverlayId::from_bytes([0; 32]);
        Self {
            inner: Arc::new(HandleInner {
                config: StdRwLock::new(config),
                overlay: StdRwLock::new(None),
                state: tokio::sync::Mutex::new(state::State::new(
                    config,
                    overlay,
                    None,
                    None,
                    config.enabled,
                )),
                keypair: StdRwLock::new(None),
                local_peer: StdRwLock::new(None),
                peers: StdRwLock::new(Vec::new()),
                action_tx,
                action_rx: StdMutex::new(Some(action_rx)),
                stats: StdMutex::new(PlumtreeStats::default()),
            }),
        }
    }

    pub(crate) fn take_action_receiver(&self) -> Option<mpsc::UnboundedReceiver<Action>> {
        self.inner
            .action_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub(crate) async fn configure(
        &self,
        config: PlumtreeConfig,
        overlay: OverlayId,
        keypair: Option<&AdnlKeyPair>,
    ) -> Result<(), OverlayError> {
        config.validate()?;
        let local_peer = keypair.map(|keypair| PeerId::from_bytes(keypair.public_key.to_bytes()));
        let local_source = keypair
            .map(|keypair| wire::source_hash(&wire::local_source(keypair)))
            .transpose()?;
        *self
            .inner
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        *self
            .inner
            .overlay
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(overlay);
        *self
            .inner
            .keypair
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = keypair.copied();
        *self
            .inner
            .local_peer
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = local_peer;
        let peers = self.peers();
        let mut state = self.inner.state.lock().await;
        *state = state::State::new(config, overlay, local_source, local_peer, config.enabled);
        state.set_peers(&peers);
        self.sync_stats(&state);
        Ok(())
    }

    #[must_use]
    pub fn config(&self) -> PlumtreeConfig {
        *self
            .inner
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn stats(&self) -> PlumtreeStats {
        *self
            .inner
            .stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) async fn update_peers(&self, peers: Vec<PeerId>) {
        self.inner
            .peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&peers);
        let mut state = self.inner.state.lock().await;
        state.set_peers(&peers);
        self.sync_stats(&state);
    }

    fn peers(&self) -> Vec<PeerId> {
        self.inner
            .peers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn enqueue(&self, actions: Vec<Action>) {
        for action in actions {
            if self.inner.action_tx.send(action).is_err() {
                break;
            }
        }
    }

    fn enqueue_outcome(&self, outcome: state::ProcessOutcome) -> Option<Vec<u8>> {
        self.enqueue(outcome.actions);
        self.enqueue(outcome.repairs.into_iter().map(Action::Repair).collect());
        outcome.payload
    }

    fn sync_stats(&self, state: &state::State) {
        *self
            .inner
            .stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state.stats();
    }

    pub(crate) async fn process(
        &self,
        from: PeerId,
        broadcast: OverlayBroadcast,
        origin: PayloadOrigin,
    ) -> Result<Option<Vec<u8>>, OverlayError> {
        if !self.config().enabled {
            return Ok(None);
        }
        let mut state = self.inner.state.lock().await;
        let result = state.process(from, broadcast, origin);
        match result {
            Ok(outcome) => {
                let payload = self.enqueue_outcome(outcome);
                self.sync_stats(&state);
                Ok(payload)
            }
            Err(error) => {
                state.note_validation_rejection();
                self.sync_stats(&state);
                Err(error)
            }
        }
    }

    pub(crate) async fn process_repair_response(
        &self,
        from: PeerId,
        key: RepairKey,
        data: &[u8],
    ) -> Result<Option<Vec<u8>>, OverlayError> {
        if !self.config().enabled {
            return Ok(None);
        }
        let mut state = self.inner.state.lock().await;
        let result = state.process_repair_response(from, key, data);
        match result {
            Ok(outcome) => {
                let payload = self.enqueue_outcome(outcome);
                self.sync_stats(&state);
                Ok(payload)
            }
            Err(error) => {
                state.note_validation_rejection();
                self.sync_stats(&state);
                Err(error)
            }
        }
    }

    pub(crate) async fn broadcast_simple(
        &self,
        id: PlumtreeBroadcastId,
        payload: Vec<u8>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        if !self.config().enabled {
            return Err(OverlayError::PlumtreeUnavailable(
                "Plumtree is disabled in the overlay configuration".to_owned(),
            ));
        }
        if id.as_bytes() == [0; 32] {
            return Err(OverlayError::InvalidPlumtreePayload(
                "empty Plumtree broadcast id".to_owned(),
            ));
        }
        let keypair = self.keypair()?;
        let timestamp = wire::unix_seconds();
        let (_, part) =
            wire::simple_signed_part(&keypair, flags, timestamp, id.as_int256(), &payload)?;
        let mut state = self.inner.state.lock().await;
        let actions = state.start_simple(id, flags, timestamp, payload, part)?;
        self.sync_stats(&state);
        let sent = actions.len();
        drop(state);
        self.enqueue(actions);
        Ok(sent)
    }

    pub(crate) async fn broadcast_fec(
        &self,
        payload: Vec<u8>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        if !self.config().enabled {
            return Err(OverlayError::PlumtreeUnavailable(
                "Plumtree is disabled in the overlay configuration".to_owned(),
            ));
        }
        let keypair = self.keypair()?;
        let config = self.config();
        let encoding = wire::encode_fec(&payload, flags, &config)?;
        let timestamp = wire::unix_seconds();
        let mut parts = Vec::with_capacity(encoding.symbols.len());
        for (part_index, symbol) in encoding.symbols.iter().enumerate() {
            let part_index = u32::try_from(part_index).map_err(|_| {
                OverlayError::InvalidPlumtreePayload("Plumtree part index overflow".to_owned())
            })?;
            let (_, part) = wire::fec_signed_part(
                &keypair,
                wire::FecPartSpec {
                    flags,
                    timestamp,
                    id: encoding.broadcast_id.clone(),
                    full_data_hash: encoding.full_data_hash.clone(),
                    full_data_size: i32::try_from(payload.len()).map_err(|_| {
                        OverlayError::InvalidPlumtreePayload(
                            "Plumtree payload is too large".to_owned(),
                        )
                    })?,
                    part_index,
                },
                symbol,
            )?;
            parts.push(part);
        }
        let mut state = self.inner.state.lock().await;
        let actions = state.start_fec(state::FecStart {
            id: PlumtreeBroadcastId::from(encoding.broadcast_id),
            flags,
            full_data_hash: encoding.full_data_hash,
            full_data_size: u32::try_from(payload.len()).map_err(|_| {
                OverlayError::InvalidPlumtreePayload("Plumtree payload is too large".to_owned())
            })?,
            part_size: encoding.part_size,
            source_count: encoding.source_count,
            parts,
        })?;
        self.sync_stats(&state);
        let sent = actions.len();
        drop(state);
        self.enqueue(actions);
        Ok(sent)
    }

    pub(crate) async fn answer_repair(
        &self,
        from: PeerId,
        broadcast_id: Int256,
        timestamp: f64,
        part_index: i32,
        tree_index: i32,
    ) -> Result<Option<Vec<u8>>, OverlayError> {
        if !self.config().enabled {
            return Ok(None);
        }
        let mut state = self.inner.state.lock().await;
        let result = state.answer_repair(from, &broadcast_id, timestamp, part_index, tree_index);
        self.sync_stats(&state);
        result
    }

    #[cfg(test)]
    pub(crate) async fn advertise_for_test(&self, key: RepairKey, peer: PeerId) -> Option<Vec<u8>> {
        let mut state = self.inner.state.lock().await;
        let wire = state.advertise_for_test(key, peer);
        self.sync_stats(&state);
        wire
    }

    pub(crate) async fn due_repairs(&self, now: Instant) -> Vec<RepairRequest> {
        if !self.config().enabled {
            return Vec::new();
        }
        let mut state = self.inner.state.lock().await;
        let result = state.due_repairs(now);
        self.sync_stats(&state);
        result
    }

    pub(crate) async fn finish_repair(&self, key: RepairKey) {
        let mut state = self.inner.state.lock().await;
        state.finish_repair(key);
        self.sync_stats(&state);
    }

    pub(crate) async fn tick(&self, now: Instant) {
        if !self.config().enabled {
            return;
        }
        let mut state = self.inner.state.lock().await;
        state.tick(now);
        self.sync_stats(&state);
    }

    pub(crate) fn repair_query(&self, key: RepairKey) -> Result<Vec<u8>, OverlayError> {
        let overlay = *self
            .inner
            .overlay
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .ok_or_else(|| {
                OverlayError::PlumtreeUnavailable(
                    "Plumtree overlay id is not configured".to_owned(),
                )
            })?;
        Ok(wire::repair_query_wire(overlay, key))
    }

    fn keypair(&self) -> Result<AdnlKeyPair, OverlayError> {
        (*self
            .inner
            .keypair
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
        .ok_or(OverlayError::MissingLocalKey)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        adnl::AdnlKeyPair,
        overlay::{
            OverlayError, OverlayId, PeerId,
            inbound::parse_repair_query,
            tl::{OverlayBroadcast, OverlayCertificate},
        },
        tl::TlInt256 as Int256,
    };

    use super::{Action, PayloadOrigin, PlumtreeBroadcastId, PlumtreeConfig, PlumtreeHandle, wire};

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
            certificate: OverlayCertificate::Empty,
            payload_timestamp: payload_timestamp.into(),
            data_size: part.data_size.cast_signed(),
            data_hash: part.data_hash,
            signature: part.signature,
        }
    }

    #[tokio::test]
    async fn disabled_handle_is_inert() {
        let handle = PlumtreeHandle::new(PlumtreeConfig {
            enabled: false,
            ..config()
        });
        let local_keypair = keypair();
        handle
            .configure(
                PlumtreeConfig {
                    enabled: false,
                    ..config()
                },
                OverlayId::from_name(b"plumtree-disabled"),
                Some(&local_keypair),
            )
            .await
            .unwrap();

        assert!(matches!(
            handle
                .broadcast_simple(PlumtreeBroadcastId::new([1; 32]), b"x".to_vec(), 0)
                .await,
            Err(OverlayError::PlumtreeUnavailable(_))
        ));
        assert!(matches!(
            handle.broadcast_fec(b"x".to_vec(), 0).await,
            Err(OverlayError::PlumtreeUnavailable(_))
        ));
        assert_eq!(handle.stats(), super::PlumtreeStats::default());

        let id = Int256([2; 32]);
        let sender = keypair();
        let message = ihave(&sender, &id, b"x", wire::unix_seconds());
        let outcome = handle
            .process(PeerId::from_bytes([1; 32]), message, PayloadOrigin::Push)
            .await
            .unwrap();
        assert!(outcome.is_none());
        assert!(
            handle
                .due_repairs(std::time::Instant::now())
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn configure_rejects_invalid_tuning() {
        let handle = PlumtreeHandle::new(config());
        let keypair = keypair();
        assert!(matches!(
            handle
                .configure(
                    PlumtreeConfig { k: 0, ..config() },
                    OverlayId::from_name(b"plumtree-invalid"),
                    Some(&keypair),
                )
                .await,
            Err(OverlayError::Configuration(_))
        ));

        let disabled = PlumtreeConfig {
            enabled: false,
            k: 0,
            ..config()
        };
        handle
            .configure(
                disabled,
                OverlayId::from_name(b"plumtree-invalid"),
                Some(&keypair),
            )
            .await
            .expect("a disabled configuration skips tuning validation");
    }

    #[tokio::test]
    async fn broadcast_simple_queues_one_frame_and_updates_stats() {
        let keypair = keypair();
        let peer = PeerId::from_bytes([1; 32]);
        let handle = PlumtreeHandle::new(config());
        handle.update_peers(vec![peer]).await;
        handle
            .configure(
                config(),
                OverlayId::from_name(b"plumtree-handle-simple"),
                Some(&keypair),
            )
            .await
            .unwrap();
        let mut actions = handle.take_action_receiver().expect("receiver is fresh");

        let id = PlumtreeBroadcastId::new([0x42; 32]);
        let payload = b"handle simple payload".to_vec();
        let sent = handle
            .broadcast_simple(id, payload.clone(), 0)
            .await
            .unwrap();
        assert_eq!(sent, 1);

        let Action::Send {
            peer: sent_to,
            wire,
        } = actions.try_recv().expect("one frame must be queued")
        else {
            panic!("the queued action must be a payload send");
        };
        assert_eq!(sent_to, peer);
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
        assert!(matches!(
            broadcast,
            OverlayBroadcast::PlumtreeSimple { ref data, .. } if data == &payload
        ));

        let stats = handle.stats();
        assert_eq!(stats.parts_sent, 1);
        assert_eq!(stats.completed_broadcasts, 1);
        assert_eq!(stats.validation_rejections, 0);

        assert_eq!(
            handle.broadcast_simple(id, payload, 0).await.unwrap(),
            0,
            "re-broadcasting the same id is a no-op"
        );
    }

    #[tokio::test]
    async fn broadcast_fec_queues_one_frame_per_part() {
        let keypair = keypair();
        let peer = PeerId::from_bytes([1; 32]);
        let handle = PlumtreeHandle::new(config());
        handle.update_peers(vec![peer]).await;
        handle
            .configure(
                config(),
                OverlayId::from_name(b"plumtree-handle-fec"),
                Some(&keypair),
            )
            .await
            .unwrap();
        let mut actions = handle.take_action_receiver().expect("receiver is fresh");

        let payload = (0..300u32)
            .map(|value| u8::try_from(value % 251).unwrap())
            .collect::<Vec<_>>();
        let sent = handle.broadcast_fec(payload, 0).await.unwrap();
        assert_eq!(sent, config().parts as usize);

        let mut part_indices = Vec::new();
        while let Ok(Action::Send {
            peer: sent_to,
            wire,
        }) = actions.try_recv()
        {
            assert_eq!(sent_to, peer);
            let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
            let OverlayBroadcast::PlumtreeFec { part_index, .. } = broadcast else {
                panic!("FEC broadcasts must queue FEC parts");
            };
            part_indices.push(part_index);
        }
        part_indices.sort_unstable();
        assert_eq!(
            part_indices,
            (0..config().parts)
                .map(u32::cast_signed)
                .collect::<Vec<_>>()
        );
        assert_eq!(handle.stats().completed_broadcasts, 1);
    }

    #[tokio::test]
    async fn process_ihave_enqueues_an_immediate_repair() {
        let handle_keypair = keypair();
        let sender = PeerId::from_bytes([1; 32]);
        let handle = PlumtreeHandle::new(config());
        handle
            .configure(
                config(),
                OverlayId::from_name(b"plumtree-handle-repair"),
                Some(&handle_keypair),
            )
            .await
            .unwrap();
        let mut actions = handle.take_action_receiver().expect("receiver is fresh");

        let id = Int256([0x77; 32]);
        let message_keypair = keypair();
        let message = ihave(&message_keypair, &id, b"repair me", wire::unix_seconds());
        let payload = handle
            .process(sender, message, PayloadOrigin::Push)
            .await
            .unwrap();
        assert!(payload.is_none());

        let Action::Repair(request) = actions.try_recv().expect("a repair must be queued") else {
            panic!("the queued action must be a repair");
        };
        assert_eq!(request.key, (id.0, 0, 0));
        assert_eq!(request.peer, sender);
        assert_eq!(handle.stats().control_received, 1);

        handle.finish_repair(request.key).await;
    }

    #[tokio::test]
    async fn repair_query_needs_a_configured_overlay() {
        let handle = PlumtreeHandle::new(config());
        let key = ([5u8; 32], 3, 4);
        assert!(matches!(
            handle.repair_query(key),
            Err(OverlayError::PlumtreeUnavailable(_))
        ));

        let keypair = keypair();
        let overlay = OverlayId::from_name(b"plumtree-handle-query");
        handle
            .configure(config(), overlay, Some(&keypair))
            .await
            .unwrap();
        let query = handle.repair_query(key).unwrap();
        let parsed = parse_repair_query(&query, overlay).expect("the query must wrap the overlay");
        assert_eq!(parsed.broadcast_id.0, key.0);
        assert_eq!(parsed.part_index, key.1.cast_signed());
        assert_eq!(parsed.tree_index, key.2.cast_signed());
    }
}
