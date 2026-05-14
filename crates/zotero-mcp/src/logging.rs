use std::path::Path;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(level: &str, log_dir: Option<&Path>) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

    if let Some(dir) = log_dir {
        std::fs::create_dir_all(dir).ok();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("zotero-mcp.log"))?;
        let file_layer = fmt::layer().with_writer(file).with_ansi(false);
        registry.with(file_layer).try_init().ok();
    } else {
        registry.try_init().ok();
    }
    Ok(())
}
