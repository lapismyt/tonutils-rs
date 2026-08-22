use std::time::Duration;

use futures::StreamExt;
use raptorq::Encoder;
use sha2::{Digest, Sha256};
use tonutils_adnl::{AdnlUdpSession, KeyPair};
use tonutils_mempool::{MempoolConfig, MempoolEvent, MempoolScannerBuilder};
use tonutils_overlay::{OverlayConfig, OverlayId, PeerId, SeedPeer};
use tonutils_tl::tl::network::{
    FecType, OverlayBroadcastFec, OverlayCertificate, PacketContents, TonNodeExternalMessage,
    TonNodeExternalMessageBroadcast,
};
use tonutils_tlb::{
    CommonMsgInfo, Either, Grams, Message as TlbMessage, MsgAddressExt, MsgAddressInt, TlbSerialize,
};
use tonutils_tvm::{Address, Builder, serialize_boc};

#[tokio::test]
async fn explicit_seed_udp_overlay_fec_reaches_scanner_stream() {
    let client_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let server_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let overlay = OverlayId::from_name(b"mempool-seed-test");
    let seed = SeedPeer {
        peer: PeerId::from_bytes(server_key.public_key.to_bytes()),
        address: server_addr.to_string(),
    };
    let config = MempoolConfig {
        event_queue_capacity: 8,
        ..Default::default()
    };
    let (scanner, manager, stream) = MempoolScannerBuilder::new()
        .download_config(false)
        .overlay_config(OverlayConfig {
            queue_capacity: 8,
            ..Default::default()
        })
        .overlay_id(overlay)
        .config(config)
        .seed(seed)
        .native_udp_seeds_only(client_addr, client_key, None)
        .start()
        .await
        .unwrap();
    drop(server_socket);
    let mut events = Box::pin(stream);
    let mut sender =
        AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
            .await
            .unwrap();
    let mut body = Builder::new();
    body.store_u32(0xfeed_beef).unwrap();
    let message = TlbMessage {
        info: CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(Address::new(0, [0x22; 32])),
            import_fee: Grams::from(0),
        },
        init: None,
        body: Either::Left(body.build().unwrap()),
    };
    let boc = serialize_boc(&message.to_cell().unwrap(), false).unwrap();
    let external = tl_proto::serialize(TonNodeExternalMessageBroadcast {
        message: TonNodeExternalMessage { data: boc.clone() },
    });
    let encoder = Encoder::with_defaults(&external, 128);
    let fec_config = encoder.get_config();
    let data_hash: [u8; 32] = Sha256::digest(&external).into();
    let symbols_count = (external.len() as u64).div_ceil(fec_config.symbol_size() as u64) as i32;
    for (seqno, packet) in encoder.get_encoded_packets(0).into_iter().enumerate() {
        let mut overlay_payload = Vec::new();
        overlay_payload.extend_from_slice(&0x75252420u32.to_le_bytes());
        overlay_payload.extend_from_slice(&overlay.as_bytes());
        overlay_payload.extend(tl_proto::serialize(OverlayBroadcastFec {
            src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![1] },
            certificate: OverlayCertificate::Empty,
            data_hash: tonutils_tl::Int256(data_hash),
            data_size: external.len() as i32,
            flags: 0,
            data: packet.split().1,
            seqno: seqno as i32,
            fec: FecType::RaptorQ {
                data_size: external.len() as i32,
                symbol_size: fec_config.symbol_size() as i32,
                symbols_count,
            },
            date: 0,
            signature: Vec::new(),
        }));
        sender
            .send_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: Some(tonutils_tl::Message::Custom {
                    data: overlay_payload,
                }),
                messages: None,
                address: None,
                priority_address: None,
                seqno: None,
                confirm_seqno: None,
                recv_addr_list_version: None,
                recv_priority_addr_list_version: None,
                reinit_date: None,
                dst_reinit_date: None,
                signature: None,
                rand2: vec![0; 7],
            })
            .await
            .unwrap();
    }
    let event = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(event) = events.next().await
                && matches!(event, MempoolEvent::ExternalMessage { .. })
            {
                break event;
            }
        }
    })
    .await
    .unwrap();
    match event {
        MempoolEvent::ExternalMessage { raw_boc, .. } => assert_eq!(raw_boc.as_ref(), boc),
        _ => unreachable!(),
    }
    assert_eq!(scanner.metrics().accepted, 1);
    manager.shutdown_wait().await;
}
