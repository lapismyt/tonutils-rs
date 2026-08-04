use tonutils_cli::Cli;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let cli = Cli::parse_args();
    cli.execute().await?;
    Ok(())
}
