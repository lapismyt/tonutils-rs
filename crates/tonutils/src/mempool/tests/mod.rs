//! Mempool-only validation, deduplication, event, and submission tests.

mod submission;
mod validation;

use std::{sync::Arc, time::Duration};

use futures::{Stream, StreamExt, future::BoxFuture};
use tokio::sync::{Mutex, RwLock};

use crate::{
    adnl::AdnlKeyPair,
    mempool::{MempoolEvent, MempoolServer},
    overlay::{
        DiscoveryStats, OverlayBroadcastPeer, OverlayClient, OverlayConfig, OverlayId,
        OverlayMemberEndpoints, OverlaySession, OverlaySessionSender, PeerId, PeerManager,
        PeerQualityConfig, PeerQualityRegistry, PeerStatus, SeedPeer,
    },
    tlb::{CommonMsgInfo, Either, Grams, Message, MsgAddressExt, MsgAddressInt, TlbSerialize},
    tvm::{Address, Builder, serialize_boc},
};

type CapturedObjects = Arc<Mutex<Vec<Arc<[u8]>>>>;

struct CapturingSender {
    sent: CapturedObjects,
}

impl OverlaySessionSender for CapturingSender {
    fn send(&self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.sent.lock().await.push(payload);
            Ok(())
        })
    }

    fn send_object(&self, object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        self.send(object)
    }
}

struct CapturingSession {
    peer: PeerId,
    sender: Arc<CapturingSender>,
}

impl OverlaySession for CapturingSession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(std::future::pending())
    }

    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        self.sender.send(payload)
    }

    fn send_object(&mut self, object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        self.sender.send_object(object)
    }

    fn sender(&self) -> Option<Arc<dyn OverlaySessionSender>> {
        Some(self.sender.clone())
    }
}

struct TestOverlay {
    client: OverlayClient,
    manager: PeerManager,
    sent: CapturedObjects,
}

async fn test_overlay() -> TestOverlay {
    let manager = PeerManager::with_overlay(
        OverlayConfig::default(),
        OverlayId::from_name(b"mempool-test"),
    )
    .unwrap();
    manager
        .pool()
        .register_peer(PeerId::from_bytes([2; 32]))
        .await;
    let sent: CapturedObjects = Arc::new(Mutex::new(Vec::new()));
    let sender = Arc::new(CapturingSender {
        sent: Arc::clone(&sent),
    });
    manager
        .add_session(Box::new(CapturingSession {
            peer: PeerId::from_bytes([1; 32]),
            sender,
        }))
        .await;

    let quality = Arc::new(PeerQualityRegistry::new(PeerQualityConfig::default()));
    let broadcast = OverlayBroadcastPeer::new(
        Arc::new(manager.clone()),
        AdnlKeyPair::generate(&mut rand::rngs::OsRng),
    )
    .with_quality(Arc::clone(&quality));
    let client = OverlayClient::new(
        manager.clone(),
        Some(broadcast),
        quality,
        OverlayMemberEndpoints::default(),
        Arc::new(RwLock::new(Vec::<SeedPeer>::new())),
        DiscoveryStats::default(),
        64,
    );
    TestOverlay {
        client,
        manager,
        sent,
    }
}

async fn new_server(
    config: crate::mempool::MempoolConfig,
) -> (MempoolServer, crate::mempool::MempoolClient, TestOverlay) {
    let overlay = test_overlay().await;
    let (server, client) = MempoolServer::new(overlay.client.clone(), config).unwrap();
    (server, client, overlay)
}

async fn next_external_event<S>(events: &mut S) -> MempoolEvent
where
    S: Stream<Item = MempoolEvent> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.next().await {
                Some(event @ MempoolEvent::ExternalMessage { .. }) => return event,
                Some(MempoolEvent::PeerStatus(PeerStatus::Connected { .. })) => {}
                Some(other) => panic!("unexpected mempool event: {other:?}"),
                None => panic!("mempool event stream closed"),
            }
        }
    })
    .await
    .expect("mempool event timed out")
}

async fn shutdown(server: MempoolServer, overlay: TestOverlay) {
    server.shutdown_wait().await;
    overlay.manager.shutdown_wait().await;
}

fn external_boc(body: u32) -> Vec<u8> {
    let mut body_cell = Builder::new();
    body_cell.store_u32(body).unwrap();
    let message = Message {
        info: CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(Address::new(0, [0x42; 32])),
            import_fee: Grams::from(0),
        },
        init: None,
        body: Either::Left(body_cell.build().unwrap()),
    };
    serialize_boc(&message.to_cell().unwrap(), false).unwrap()
}

fn routing() -> crate::overlay::RoutingMetadata {
    crate::overlay::RoutingMetadata::new(
        OverlayId::from_name(b"mempool-test"),
        PeerId::from_bytes([2; 32]),
    )
}

fn external_wrapper(raw_boc: &[u8]) -> Vec<u8> {
    tl_proto::serialize(crate::overlay::tl::TonNodeExternalMessageBroadcast {
        message: crate::overlay::tl::TonNodeExternalMessage {
            data: raw_boc.to_vec(),
        },
    })
}

fn pool(overlay: &TestOverlay) -> Arc<crate::overlay::OverlayPeerPool> {
    overlay.manager.pool()
}
