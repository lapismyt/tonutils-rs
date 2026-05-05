use std::str::FromStr;
use tonutils::network_config::ConfigGlobal;

fn main() -> anyhow::Result<()> {
    let Some(config_json) = std::env::var("TON_GLOBAL_CONFIG_JSON").ok() else {
        eprintln!("set TON_GLOBAL_CONFIG_JSON to run this example");
        return Ok(());
    };

    let config = ConfigGlobal::from_str(&config_json)?;
    for (index, liteserver) in config.liteservers.iter().enumerate() {
        println!("{index}: {}", liteserver.socket_addr());
    }
    Ok(())
}
