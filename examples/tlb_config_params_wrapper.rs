use tonutils::tlb::{ConfigParams, TlbDeserialize, TlbSerialize};
use tonutils::tvm::{Builder, Dict, DictValue};

fn main() -> anyhow::Result<()> {
    let config_value = {
        let mut builder = Builder::new();
        builder.store_u32(0x0fac_ade0)?;
        builder.build()?
    };

    let mut raw_config = Dict::new(32);
    raw_config.set_int_key(0, DictValue::Cell(config_value))?;
    let raw_config_root = raw_config
        .serialize()?
        .expect("offline config dictionary is non-empty");

    let params = ConfigParams {
        config_addr: [0x55; 32],
        config: raw_config_root,
    };

    let cell = params.to_cell()?;
    let decoded = ConfigParams::from_cell(cell)?;

    println!(
        "config_addr={} root_hash={}",
        hex::encode(decoded.config_addr),
        hex::encode(decoded.config.hash())
    );

    Ok(())
}
