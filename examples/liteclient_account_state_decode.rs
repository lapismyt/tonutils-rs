use tonutils::liteclient::boc::decode_account_state_boc;
use tonutils::tlb::{Account, TlbSerialize};
use tonutils::tvm::serialize_boc;

fn main() -> anyhow::Result<()> {
    let raw = match std::env::var("TON_ACCOUNT_STATE_BOC_HEX") {
        Ok(hex) => hex::decode(hex.trim())?,
        Err(_) => {
            println!(
                "TON_ACCOUNT_STATE_BOC_HEX is not set; using an offline Account::None fixture"
            );
            serialize_boc(&Account::None.to_cell()?, false)?
        }
    };

    let decoded = decode_account_state_boc(raw)?;
    println!("root_hash={}", decoded.boc.root_hash_hex());
    println!("account={:?}", decoded.account);
    Ok(())
}
