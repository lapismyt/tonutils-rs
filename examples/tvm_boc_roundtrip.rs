use tonutils::tvm::{Builder, deserialize_boc, serialize_boc};

fn main() -> anyhow::Result<()> {
    let mut payload = Builder::new();
    payload.store_u32(0xfeed_beef)?;
    let payload = payload.build()?;

    let mut root = Builder::new();
    root.store_u64(0x1122_3344_5566_7788)?.store_ref(payload)?;
    let root = root.build()?;

    let boc = serialize_boc(&root, true)?;
    let decoded = deserialize_boc(&boc)?;

    println!(
        "boc_len={} root_bits={} refs={}",
        boc.len(),
        decoded.bit_len(),
        decoded.reference_count()
    );

    Ok(())
}
