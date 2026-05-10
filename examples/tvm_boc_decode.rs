use tonutils::tlb::{Account, TlbDeserialize, TlbSerialize};
use tonutils::tvm::{boc_to_hex, hex_to_boc};

fn main() -> anyhow::Result<()> {
    let fixture = Account::None.to_cell()?;
    let hex = boc_to_hex(&fixture, false)?;
    let decoded = hex_to_boc(&hex)?;
    let account = Account::from_cell(decoded.clone())?;

    println!("boc_hex={hex}");
    println!("root_hash={}", hex::encode(decoded.hash()));
    println!("account={account:?}");
    Ok(())
}
