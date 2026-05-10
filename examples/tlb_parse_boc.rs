use tonutils::tlb::{Account, TlbDeserialize, TlbSerialize};
use tonutils::tvm::{boc_to_hex, hex_to_boc};

fn main() -> anyhow::Result<()> {
    let fixture = Account::None.to_cell()?;
    let hex =
        std::env::var("TON_TLB_BOC_HEX").unwrap_or_else(|_| boc_to_hex(&fixture, false).unwrap());
    let cell = hex_to_boc(&hex)?;
    let account = Account::from_cell(cell.clone())?;
    let roundtrip = account.to_cell()?;

    println!(
        "account={:?} root_hash={} roundtrip_hash={}",
        account,
        hex::encode(cell.hash()),
        hex::encode(roundtrip.hash())
    );
    Ok(())
}
