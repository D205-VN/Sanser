use sanser_server::config::Config;
use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

fn default_log_filter() -> Targets {
    Targets::new()
        .with_target("sanser_server", LevelFilter::INFO)
        .with_target("tower_http", LevelFilter::INFO)
}

fn log_filter() -> Targets {
    let Ok(value) = std::env::var("RUST_LOG") else {
        return default_log_filter();
    };
    if value.trim().is_empty() {
        return default_log_filter();
    }
    value.parse::<Targets>().unwrap_or_else(|error| {
        eprintln!("Sanser server: ignoring invalid RUST_LOG filter: {error}");
        default_log_filter()
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    if let Err(error) = tracing_subscriber::registry()
        .with(log_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_target(true)
                .with_ansi(false),
        )
        .try_init()
    {
        eprintln!("Sanser server: logging could not initialize: {error}");
    }
    let config = Config::from_env()?;
    tracing::info!("server configuration is valid");
    sanser_server::serve(config).await?;
    Ok(())
}
