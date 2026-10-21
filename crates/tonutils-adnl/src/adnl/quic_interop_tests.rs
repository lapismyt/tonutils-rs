//! Env-gated interoperability test for the pairing upstream's
//! `test-quic-interop.sh` matrix never exercised (TODO:436): the quictls/C++
//! `quic-example-client` connecting to *our* rustls QUIC server with a
//! mandatory client raw public key.
//!
//! The test is `#[ignore]`d and runs only when `QUIC_INTEROP_CLIENT` points
//! at a built `quic-example-client` binary; CI wires that up in
//! `.github/workflows/quic-interop.yml`. The server side reuses the
//! production TLS pieces ([`TonRpkClientVerifier`], [`SniRpkResolver`],
//! the raw-public-key certified key) through
//! `make_quinn_server_config_with_alpn`; the only profile difference from a
//! production server is additionally offering the upstream demo ALPN
//! `hq-interop` (the example client hardcodes it and cannot send `ton`).
//!
//! What the pairing proves end to end:
//! - quictls presents a self-signed Ed25519 raw public key certificate and
//!   our mandatory client verifier accepts it inside the handshake;
//! - the demo client sends no SNI (it has no expected peer key to derive
//!   one from), and the server serves its default identity the way upstream
//!   `sni_select_cb` does;
//! - after the handshake the raw demo protocol (HTTP/0.9 `GET` on a stream,
//!   response bytes plus stream end) flows over the connection.

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use super::crypto::KeyPair as AdnlKeyPair;
use super::quic_tls::make_quinn_server_config_with_alpn;

/// ALPN the upstream `quic-example-client` hardcodes (its `--alpn` default
/// with no flag to change it; `quic/quic-example-client.cpp`).
const DEMO_ALPN: &[u8] = b"hq-interop";

/// Response bytes the demo client prints before declaring success on the
/// stream end.
const DEMO_RESPONSE: &[u8] = b"tonutils-rs quic interop response\n";

/// Serves one connection the way the upstream interop servers do: read the
/// client's request stream to its end, then answer with bytes followed by a
/// stream end (the client prints the bytes and logs `test passed` on FIN).
async fn serve_demo_connection(connection: &quinn::Connection) {
    let request = tokio::select! {
        accepted = connection.accept_uni() => {
            let mut recv = accepted.expect("client uni stream");
            let request = recv
                .read_to_end(1024)
                .await
                .expect("read demo request");
            let mut send = connection
                .open_uni()
                .await
                .expect("open response stream");
            send.write_all(DEMO_RESPONSE)
                .await
                .expect("write response");
            send.finish().expect("finish response stream");
            request
        }
        accepted = connection.accept_bi() => {
            let (mut send, mut recv) = accepted.expect("client bidi stream");
            let request = recv
                .read_to_end(1024)
                .await
                .expect("read demo request");
            send.write_all(DEMO_RESPONSE)
                .await
                .expect("write response");
            send.finish().expect("finish response stream");
            request
        }
    };
    assert!(
        request.starts_with(b"GET /"),
        "the demo client must send an HTTP/0.9 GET, got {request:?}"
    );
}

#[tokio::test]
#[ignore = "needs the upstream quic-example-client binary (QUIC_INTEROP_CLIENT)"]
async fn cpp_quictls_client_completes_the_demo_pairing() {
    let client_bin = std::env::var("QUIC_INTEROP_CLIENT").expect(
        "set QUIC_INTEROP_CLIENT to the upstream quic-example-client binary \
         (see .github/workflows/quic-interop.yml)",
    );

    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

    // The production SNI and client-auth pieces, with the demo ALPN added so
    // the hardcoded `hq-interop` client can negotiate; `ton` stays offered
    // for symmetry with the production profile.
    let server_config = make_quinn_server_config_with_alpn(&keypair, &[DEMO_ALPN, b"ton"])
        .expect("interop server TLS config");

    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind interop port");
    socket.set_nonblocking(true).expect("nonblocking socket");
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_config),
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .expect("server endpoint");
    let port = endpoint.local_addr().expect("local addr").port();

    let mut client = Command::new(&client_bin)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the C++ interop client");

    let serve = async {
        let incoming = tokio::time::timeout(Duration::from_secs(30), endpoint.accept())
            .await
            .expect("the C++ client must reach the server")
            .expect("the server endpoint must stay open");
        let connecting = incoming.accept().expect("incoming connection");
        let connection = tokio::time::timeout(Duration::from_secs(30), connecting)
            .await
            .expect("the handshake must complete within 30s")
            .expect("the raw-public-key handshake must succeed");

        let observed_sni = connection
            .handshake_data()
            .expect("handshake data")
            .downcast_ref::<quinn::crypto::rustls::HandshakeData>()
            .and_then(|data| data.server_name.clone());
        assert_eq!(
            observed_sni, None,
            "the demo client has no expected peer key and sends no SNI; the \
             server must serve its default identity like upstream sni_select_cb"
        );

        serve_demo_connection(&connection).await;

        // Keep the connection alive until the client consumed the response.
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    tokio::time::timeout(Duration::from_secs(60), serve)
        .await
        .expect("serving the demo pairing must finish within 60s");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = client.try_wait().expect("poll the C++ client") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the C++ client did not exit after the response"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = client.wait_with_output().expect("collect client output");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("quic-example-client output:\n{combined}");

    assert!(status.success(), "the C++ client must exit cleanly");
    assert!(
        combined.contains("test passed"),
        "the C++ client must report test passed:\n{combined}"
    );
}
