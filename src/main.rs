use tonutils_rs::cli::Cli;
use tonutils_rs::utils::init_logger;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger().unwrap();
    let cli = Cli::parse_args();
    cli.execute().await?;
    Ok(())
}
