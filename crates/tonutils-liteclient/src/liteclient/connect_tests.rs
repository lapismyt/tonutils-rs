use std::io::ErrorKind;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tonutils_adnl::crypto::{KeyPair, SecretKey};
use tonutils_adnl::{AdnlError, AdnlPeer};

use super::client::LiteClient;
use super::types::LiteError;

fn server_keypair() -> KeyPair {
    KeyPair::from(&SecretKey::from_bytes([7; 32]))
}

async fn bind_loopback() -> (TcpListener, std::net::SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    (listener, address)
}

#[tokio::test]
async fn connect_accepts_loopback_adnl_handshake() {
    let (listener, address) = bind_loopback().await;
    let server_keypair = server_keypair();
    let server_public = server_keypair.public_key.to_bytes();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        AdnlPeer::handle_handshake(stream, move |_| Some(server_keypair)).await
    });

    LiteClient::connect(address, server_public).await.unwrap();

    assert!(server.await.unwrap().is_ok());
}

#[tokio::test]
async fn connect_rejects_invalid_public_key_without_masking_adnl_error() {
    let (listener, address) = bind_loopback().await;
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    let invalid_public_key: &[u8] = &[];
    let error = match LiteClient::connect(address, invalid_public_key).await {
        Ok(_) => panic!("connect should reject an invalid public key"),
        Err(error) => error,
    };

    assert!(
        matches!(error, LiteError::AdnlError(AdnlError::InvalidPublicKey)),
        "unexpected error: {error:?}"
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn connect_reports_end_of_stream_before_handshake_acknowledgement() {
    let (listener, address) = bind_loopback().await;
    let server_keypair = server_keypair();
    let server_public = server_keypair.public_key.to_bytes();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut handshake = [0; 256];
        stream.read_exact(&mut handshake).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let error = match LiteClient::connect(address, server_public).await {
        Ok(_) => panic!("connect should reject a closed handshake"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        LiteError::AdnlError(AdnlError::EndOfStream)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn connect_with_timeout_reports_handshake_timeout() {
    let (listener, address) = bind_loopback().await;
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    let server_public = server_keypair().public_key.to_bytes();
    let timeout = Duration::from_millis(10);
    let error = match LiteClient::connect_with_timeout(address, server_public, timeout).await {
        Ok(_) => panic!("connect should time out while waiting for handshake"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        LiteError::AdnlError(AdnlError::Timeout {
            operation: "adnl_handshake",
            timeout: actual_timeout,
        }) if actual_timeout == timeout
    ));
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn connect_reports_refused_loopback_connection_as_adnl_io_error() {
    let (listener, address) = bind_loopback().await;
    drop(listener);

    let server_public = server_keypair().public_key.to_bytes();
    let error = match LiteClient::connect(address, server_public).await {
        Ok(_) => panic!("connect should fail for a refused connection"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        LiteError::AdnlError(AdnlError::IoError(error))
            if error.kind() == ErrorKind::ConnectionRefused
    ));
}
