use crate::state::AppState;

pub async fn read_all(state: &AppState) -> anyhow::Result<String> {
    let ts = zotero_core::reader::tags::list(&state.pool, 1, None).await?;
    Ok(serde_json::to_string_pretty(&ts)?)
}
