//! Broadcasts a V5R1 wallet transfer onto the TON overlay as fast as
//! possible: builds the signed external message locally, starts the
//! UDP-backed scanner, waits for the first overlay session, and fires
//! `send_external` - the overlay-direct path that fans the signed broadcast
//! out to every session in parallel, ranked by delivery quality.
//!
//! ```text
//! cargo run --release --example mempool_send_v5r1 -- \
//!     "<24-word mnemonic phrase>" 0:<dest_address_hex> 1000000 0
//! ```
//!
//! Arguments: mnemonic phrase (one quoted argument), destination address,
//! amount in nanoton, and the wallet `seqno` (`0` for a fresh wallet).
//! `TON_NETWORK=testnet` switches both the wallet id and the scanner
//! overlay.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tonutils_adnl::{KeyPair, now_i32};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_tvm::Address;
use tonutils_wallet::{
    MAINNET_GLOBAL_ID, TESTNET_GLOBAL_ID, TonMnemonic, WalletMessage, WalletV5R1,
    WalletV5R1WalletId, wallet_v5r1_code,
};

/// Mainnet zero state file hash (also the testnet one; only the network
/// global id and the bootstrap config differ).
const MAINNET_ZERO_STATE_HASH: [u8; 32] = [
    0x5e, 0x99, 0x4f, 0xcf, 0x4d, 0x42, 0x5c, 0x0a, 0x6c, 0xe6, 0xa7, 0x92, 0x59, 0x4b, 0x71, 0x73,
    0x20, 0x5f, 0x74, 0x0a, 0x39, 0xcd, 0x56, 0xf5, 0x37, 0xde, 0xfd, 0x28, 0xb4, 0x8a, 0x0f, 0x6e,
];

const USAGE: &str =
    "usage: mempool_send_v5r1 \"<mnemonic phrase>\" <dest:addr> <amount_nanoton> <seqno>";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::try_init().ok();

    let mut args = std::env::args().skip(1);
    let phrase = args.next().ok_or(USAGE)?;
    let destination = args.next().ok_or(USAGE)?;
    let amount: u64 = args.next().ok_or(USAGE)?.parse()?;
    let seqno: u32 = args.next().ok_or(USAGE)?.parse()?;

    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");
    let global_id = if testnet {
        TESTNET_GLOBAL_ID
    } else {
        MAINNET_GLOBAL_ID
    };

    // Build the signed external message before touching the network.
    let started = Instant::now();
    let mnemonic = TonMnemonic::from_phrase(&phrase, None)?;
    let wallet_id = WalletV5R1WalletId::client(global_id, 0, 0, 0).pack()?;
    let wallet = WalletV5R1::new(mnemonic.public_key(), wallet_id, wallet_v5r1_code()?, 0);
    let transfer = WalletMessage::internal(Address::from_str(&destination)?, amount).with_mode(3);
    let valid_until = u32::try_from(now_i32())
        .unwrap_or(u32::MAX)
        .saturating_add(60);
    // `state_init` is included so an undeployed (seqno 0) wallet deploys on
    // this very message; an already deployed wallet ignores it.
    let boc = wallet.build_external_message_boc(
        seqno,
        valid_until,
        vec![transfer],
        mnemonic.signing_key(),
        true,
    )?;
    println!(
        "wallet {} | transfer {} nanoton | seqno {seqno} | built in {:.1}ms",
        wallet.address()?.to_raw(),
        amount,
        started.elapsed().as_secs_f64() * 1000.0,
    );

    // Start the UDP-backed scanner; `send_external` publishes straight onto
    // the overlay sessions, no liteserver is involved.
    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;
    let connect_started = Instant::now();
    let (scanner, manager, _stream) = MempoolScannerBuilder::new()
        .testnet(testnet)
        .native_udp_for_shard_public(
            local_addr,
            local_key,
            0,
            i64::MIN,
            MAINNET_ZERO_STATE_HASH,
            None,
        )
        .start()
        .await?;
    println!(
        "bootstrap done in {:.1}ms, waiting for the first overlay session...",
        connect_started.elapsed().as_secs_f64() * 1000.0
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while manager.peer_count().await == 0 {
        if Instant::now() >= deadline {
            return Err("no overlay session connected within 30s".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let send_started = Instant::now();
    let hash = scanner.send_external(boc).await?;
    println!(
        "broadcast {} to {} session(s) in {:.1}ms",
        hex::encode(hash),
        manager.peer_count().await,
        send_started.elapsed().as_secs_f64() * 1000.0,
    );

    manager.shutdown_wait().await;
    Ok(())
}
