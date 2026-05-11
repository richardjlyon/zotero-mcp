use crate::state::AppState;

pub async fn read_all(state: &AppState) -> anyhow::Result<String> {
    let cs = zotero_core::reader::collections::list(&state.pool, 1, None).await?;
    Ok(serde_json::to_string_pretty(&cs)?)
}
