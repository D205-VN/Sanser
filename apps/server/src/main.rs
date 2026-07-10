use sanser_server::config::Config;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("sanser_server=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer().json().with_target(true))
        .init();

    let config = Config::from_env()?;
    sanser_server::serve(config).await?;
    Ok(())
}
