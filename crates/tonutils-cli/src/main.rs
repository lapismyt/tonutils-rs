//! Binary entry point for the `tonutils` command-line tool.
//!
//! Use `tonutils --help` to inspect the stable command groups. Offline TVM,
//! BoC, and schema commands are suitable for CI; network and wallet-send
//! commands require explicit configuration and must be reviewed before use.

use tonutils_cli::Cli;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let cli = Cli::parse_args();
    cli.execute().await?;
    Ok(())
}
