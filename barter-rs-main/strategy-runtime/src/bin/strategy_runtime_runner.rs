use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    info!("strategy-runtime runner stub: configuration and redis loop not implemented yet");
}
