use std::sync::Arc;

use tonutils::tlb::MerkleProof;
use tonutils::tvm::{Cell, deserialize_boc, serialize_boc};

fn main() -> anyhow::Result<()> {
    let raw = match std::env::var("TON_MERKLE_PROOF_BOC_HEX") {
        Ok(hex) => hex::decode(hex.trim())?,
        Err(std::env::VarError::NotPresent) => offline_merkle_proof_boc()?,
        Err(err) => return Err(err.into()),
    };

    let proof = MerkleProof::from_exotic_cell(deserialize_boc(&raw)?)?;
    println!("virtual_hash={}", hex::encode(proof.virtual_hash));
    println!("child_hash_matches={}", proof.verify_virtual_hash());
    Ok(())
}

fn offline_merkle_proof_boc() -> anyhow::Result<Vec<u8>> {
    let child = Arc::new(Cell::with_data(vec![0xAA], 8)?);
    let mut data = vec![0x03];
    data.extend_from_slice(&child.hash());
    data.extend_from_slice(&child.depth().to_be_bytes());
    let proof = Arc::new(Cell::with_exotic_data(data, 280, vec![child])?);
    serialize_boc(&proof, false)
}
