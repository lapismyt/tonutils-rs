use tonutils::cli::Cli;
use tonutils::utils::init_logger;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger().unwrap();
    let cli = Cli::parse_args();
    cli.execute().await?;
    Ok(())
}
