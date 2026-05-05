use tonutils::adnl::crypto::KeyPair;
use tonutils::adnl::{AdnlAddress, AdnlBuilder, AdnlHandshake};

fn main() -> anyhow::Result<()> {
    let server = KeyPair::generate(&mut rand::rngs::OsRng);
    let client = KeyPair::generate(&mut rand::rngs::OsRng);

    let handshake = AdnlBuilder::with_random_aes_params(&mut rand::rngs::OsRng)
        .perform_ecdh(&client, &server.public_key);

    let packet = handshake.to_bytes();
    let decoded = AdnlHandshake::decrypt_from_raw(&packet, |address| {
        if *address == AdnlAddress::from(&server.public_key) {
            Some(server)
        } else {
            None
        }
    })?;

    println!(
        "handshake_ok sender={} receiver={}",
        decoded.sender(),
        hex::encode(decoded.receiver().as_bytes())
    );

    Ok(())
}
