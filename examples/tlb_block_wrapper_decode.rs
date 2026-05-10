use tonutils::tlb::{Block, TlbDeserialize, TlbSerialize};
use tonutils::tvm::{Builder, Cell};

fn main() -> anyhow::Result<()> {
    let block = Block {
        global_id: -239,
        info: child_cell(0x1111_1111)?,
        value_flow: child_cell(0x2222_2222)?,
        state_update: child_cell(0x3333_3333)?,
        extra: child_cell(0x4444_4444)?,
    };

    let cell = block.to_cell()?;
    let decoded = Block::from_cell(cell)?;

    println!(
        "global_id={} info={} value_flow={} state_update={} extra={}",
        decoded.global_id,
        hash_hex(&decoded.info),
        hash_hex(&decoded.value_flow),
        hash_hex(&decoded.state_update),
        hash_hex(&decoded.extra)
    );

    Ok(())
}

fn child_cell(tag: u32) -> anyhow::Result<std::sync::Arc<Cell>> {
    let mut builder = Builder::new();
    builder.store_u32(tag)?;
    Ok(builder.build()?)
}

fn hash_hex(cell: &Cell) -> String {
    hex::encode(cell.hash())
}
