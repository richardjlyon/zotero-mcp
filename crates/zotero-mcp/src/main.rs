mod logging;

fn main() -> anyhow::Result<()> {
    let config = zotero_core::Config::load()
        .unwrap_or_default();
    logging::init(&config.logging.level, Some(&config.resolved_log_dir()))?;
    tracing::info!("zotero-mcp starting (stub)");
    Ok(())
}
