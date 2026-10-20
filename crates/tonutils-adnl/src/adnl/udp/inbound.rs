//! The process-wide executor that runs the inbound ADNL datagram path.
//!
//! Latency on this path is not a quality-of-service detail: a peer keeps a
//! node it learned about in a pending set, verifies it with `overlay.ping`
//! on a five second deadline, and only promotes a node whose `overlay.pong`
//! arrives in time to a verified member that receives broadcast fan-out
//! (`OverlayImpl::check_pending_peer` in `overlay/overlay-peers.cpp`).
//! Upstream reads the socket and dispatches inbound datagrams on the
//! network manager's own receive thread (`AdnlNetworkManager` in
//! `adnl/adnl-network-manager.cpp`) for the same reason: the application's
//! threads must not be able to delay an answer that gates liveness.
//!
//! Running the demultiplexer and the session receive loops as ordinary
//! tasks on the caller's runtime ties every inbound datagram to that
//! runtime's task queue.  On the live delivery gate each of the eight
//! samples is a single-threaded runtime whose ready queue regularly holds
//! seconds of scanner work: run 39114243866 measured 15-50 % of inbound
//! datagrams waiting over a second in a session queue, up to 15 s, and
//! every session of a process stalled at the same instant (several
//! sessions logging 10.1 s waits together), while the same build answered
//! `overlay.ping` in milliseconds on an idle host and was pushed a
//! broadcast within a second.  The ping answer itself was not slow; it
//! queued behind unrelated task polls.
//!
//! Inbound work therefore gets a thread of its own: one current-thread
//! runtime for the whole process, parked unless a datagram arrives, so a
//! wakeup only has to schedule this one idle thread instead of waiting
//! behind the application's ready queue.  Transports spawn their
//! demultiplexer and sessions their receive loop onto it through
//! [`handle`], and those tasks keep using the ordinary async primitives
//! of the crate; only the executor changes.  The thread lives for the
//! process lifetime, exactly like upstream's network manager thread, and
//! an application that never completes a datagram never starts it.

use std::sync::OnceLock;

thread_local! {
    /// Set while executing on the inbound thread, so the shared-socket
    /// send path can take the synchronous non-blocking send that does
    /// not depend on the application runtime's IO driver.
    static ON_INBOUND_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the current thread as the inbound executor thread.
pub(super) fn mark_inbound_thread() {
    ON_INBOUND_THREAD.with(|flag| flag.set(true));
}

/// Whether the caller runs on the inbound executor thread.
pub(super) fn on_inbound_thread() -> bool {
    ON_INBOUND_THREAD.with(|flag| flag.get())
}

/// Handle to the inbound executor, created on its first use.
static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Returns the handle that inbound tasks are spawned on.
///
/// The first call starts the `adnl-udp-inbound` thread, which owns a
/// current-thread runtime and parks on it for the process lifetime;
/// later calls are free.  Every transport and session of the process
/// shares it, matching upstream's one receive thread per node.
pub(super) fn handle() -> &'static tokio::runtime::Handle {
    HANDLE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("adnl-udp-inbound".to_owned())
            .spawn(move || {
                mark_inbound_thread();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("the ADNL inbound runtime must build");
                let _ = tx.send(runtime.handle().clone());
                // Parks forever: the runtime stays alive and keeps
                // polling every task spawned on its handle.
                runtime.block_on(std::future::pending::<()>());
            })
            .expect("the ADNL inbound thread must start");
        rx.recv()
            .expect("the ADNL inbound thread must report its handle")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker distinguishes the inbound executor thread from every
    /// other thread: the shared-socket send path relies on it to take
    /// the synchronous non-blocking send only where the application
    /// runtime's IO driver would otherwise gate the write.
    #[test]
    fn the_marker_is_set_only_on_the_inbound_thread() {
        assert!(
            !on_inbound_thread(),
            "ordinary threads must not report the inbound marker"
        );
        // A task spawned on the handle runs on the inbound runtime's
        // own thread; `Handle::block_on` would run it here instead.
        let (tx, rx) = std::sync::mpsc::channel();
        handle().spawn(async move {
            let _ = tx.send(on_inbound_thread());
        });
        let on_inbound = rx.recv().expect("the spawned task must report");
        assert!(on_inbound, "the inbound thread must report its marker");
    }
}
