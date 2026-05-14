use crate::core::citations::{format_bibliography, format_citation};
use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FormatCitationArgs {
    pub item_key: String,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_format")]
    pub format: String,
}
fn default_style() -> String {
    "apa".into()
}
fn default_format() -> String {
    "bib".into()
}

pub async fn format_citation_t(
    s: &AppState,
    a: FormatCitationArgs,
) -> Result<CallToolResult, Error> {
    let r = format_citation(&s.api, &a.item_key, &a.style, &a.format)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text(r)]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FormatBibArgs {
    pub item_keys: Vec<String>,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_format")]
    pub format: String,
}

pub async fn format_bibliography_t(
    s: &AppState,
    a: FormatBibArgs,
) -> Result<CallToolResult, Error> {
    let r = format_bibliography(&s.api, &a.item_keys, &a.style, &a.format)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text(r)]))
}
