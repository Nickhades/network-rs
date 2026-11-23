use anyhow::Result;
use clap::Parser;
use nm_freebsd_daemon::config::{Args, Config};
use nm_freebsd_daemon::daemon;

fn init_logging(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_new(level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::from_args(args);
    init_logging(&config.log_level);
    tracing::debug!(
        desktop = config.desktop.as_str(),
        state_dir = %config.state_dir.display(),
        wpa_ctrl_dir = %config.wpa_ctrl_dir.display(),
        "starting nm-freebsd-daemon"
    );
    daemon::run(config).await
}
