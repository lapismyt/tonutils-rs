use num_bigint::{BigInt, BigUint};
use tonutils::tvm::{Builder, Slice};

fn main() -> anyhow::Result<()> {
    let mut builder = Builder::new();
    builder
        .store_uint(0xabu64, 8)?
        .store_big_uint(&BigUint::from(1u128 << 100), 128)?
        .store_big_int(&BigInt::from(-123_456_789i64), 64)?;

    let cell = builder.build()?;
    let mut slice = Slice::new(cell);

    let tag = slice.load_uint(8)?;
    let value = slice.load_big_uint(128)?;
    let signed = slice.load_big_int(64)?;

    println!("tag={tag} big_uint={value} big_int={signed}");
    Ok(())
}
