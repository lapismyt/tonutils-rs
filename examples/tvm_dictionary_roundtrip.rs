use tonutils::tvm::{Builder, Dict, DictValue, Slice};

fn main() -> anyhow::Result<()> {
    let mut dict = Dict::new(8);
    dict.set_int_key(1, DictValue::Uint(0x11, 8))?;
    dict.set_int_key(7, DictValue::Uint(0x77, 8))?;
    dict.set_int_key(42, DictValue::Uint(0x42, 8))?;

    let mut builder = Builder::new();
    builder.store_dictionary(Some(&dict))?;
    let cell = builder.build()?;

    let mut slice = Slice::new(cell.clone());
    let decoded = slice.load_dict(8)?.expect("offline fixture is non-empty");

    println!(
        "key_size={} len={} root_hash={}",
        decoded.key_size(),
        decoded.len(),
        hex::encode(cell.hash())
    );

    Ok(())
}
