mod common;

fn main() -> anyhow::Result<()> {
    let config = common::load_config()?;
    for (index, liteserver) in config.liteservers.iter().enumerate() {
        println!("{index}: {}", liteserver.socket_addr());
    }
    Ok(())
}
