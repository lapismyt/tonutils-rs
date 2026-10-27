//! Opt-in wire capture for official-node fixtures and live diagnostics.
//!
//! When armed, the receive path appends what it actually sees to a
//! JSONL file: raw datagrams as read off the socket by the receive
//! thread (`datagram`), the decoded `adnl.packetContents` bodies
//! sessions dispatch (`packetContents`), and the payload of
//! `adnl.message.custom` messages (`custom`) - the wrapper that overlay
//! broadcasts and mempool external messages ride in.  The capture is
//! armed by environment variables, read once on first use:
//!
//! - `ADNL_UDP_CAPTURE_FILE`: the JSONL file to append to; while
//!   unset every record call is a no-op and the receive path pays
//!   nothing.
//! - `ADNL_UDP_CAPTURE_MAX`: the total number of records to write
//!   (default [`DEFAULT_CAPTURE_MAX`]); later records are dropped.
//!
//! This exists for the wire fixtures in `fixtures/cross_sdk/` and for
//! live diagnostics.  Each record carries only what the socket
//! carried plus what this process already decrypted; `custom` records
//! are the decrypted inner payload and therefore plaintext, which is
//! why the capture is strictly opt-in and capped.

use std::{
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

/// The record cap when `ADNL_UDP_CAPTURE_MAX` is unset or unparseable.
const DEFAULT_CAPTURE_MAX: u64 = 64;

struct Capture {
    file: Mutex<std::fs::File>,
    remaining: AtomicU64,
}

impl Capture {
    fn record(&self, kind: &str, peer: Option<SocketAddr>, bytes: &[u8]) {
        // A load/compare_exchange loop consumes one slot per written
        // record and stops at zero without underflow (kept on the
        // long-stable atomics for the crate's MSRV).
        loop {
            let left = self.remaining.load(Ordering::Relaxed);
            if left == 0 {
                return;
            }
            if self
                .remaining
                .compare_exchange_weak(left, left - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        let peer = peer.map_or_else(|| "-".to_owned(), |addr| addr.to_string());
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(
                file,
                "{{\"kind\":\"{kind}\",\"peer\":\"{peer}\",\"at_unix_ms\":{at_ms},\"hex\":\"{}\"}}",
                hex::encode(bytes),
            );
        }
    }
}

static CAPTURE: OnceLock<Option<Capture>> = OnceLock::new();

fn capture() -> Option<&'static Capture> {
    CAPTURE
        .get_or_init(|| {
            let path = std::env::var("ADNL_UDP_CAPTURE_FILE")
                .ok()
                .filter(|path| !path.is_empty())?;
            let max = std::env::var("ADNL_UDP_CAPTURE_MAX")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_CAPTURE_MAX);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()?;
            log::info!("ADNL wire capture armed: path={path} max={max}");
            Some(Capture {
                file: Mutex::new(file),
                remaining: AtomicU64::new(max),
            })
        })
        .as_ref()
}

/// Whether a capture is armed.
///
/// The receive path uses it to skip serializing payloads it would
/// otherwise throw away.
pub(super) fn armed() -> bool {
    capture().is_some()
}

/// Appends one capture record.
///
/// A no-op when no capture is armed or the record cap has been
/// reached.
pub(super) fn record(kind: &str, peer: Option<SocketAddr>, bytes: &[u8]) {
    if let Some(capture) = capture() {
        capture.record(kind, peer, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture writes JSONL records, stops at its cap, and leaves
    /// the file parseable: the fixture tooling downstream consumes
    /// exactly this shape.
    #[test]
    fn records_stop_at_the_cap_and_stay_line_delimited() {
        let path = std::env::temp_dir().join(format!(
            "adnl-capture-test-{}-{:?}.jsonl",
            std::process::id(),
            std::thread::current().id(),
        ));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .expect("the temp capture file must open");
        let capture = Capture {
            file: Mutex::new(file),
            remaining: AtomicU64::new(2),
        };
        let peer: SocketAddr = "127.0.0.1:30303".parse().unwrap();
        capture.record("datagram", Some(peer), &[0xaa, 0xbb]);
        capture.record("custom", None, &[0xcc]);
        capture.record("datagram", None, &[0xdd]);

        let body = std::fs::read_to_string(&path).expect("the capture file must read back");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "the third record must hit the cap");
        assert!(lines[0].contains("\"kind\":\"datagram\""), "{}", lines[0]);
        assert!(
            lines[0].contains("\"peer\":\"127.0.0.1:30303\""),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains("\"hex\":\"aabb\""), "{}", lines[0]);
        assert!(lines[1].contains("\"kind\":\"custom\""), "{}", lines[1]);
        assert!(lines[1].contains("\"peer\":\"-\""), "{}", lines[1]);
        assert!(lines[1].contains("\"hex\":\"cc\""), "{}", lines[1]);
        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each record must be one JSON object");
            assert!(parsed.get("at_unix_ms").is_some(), "{line}");
        }
        std::fs::remove_file(&path).expect("the temp capture file must clean up");
    }
}
