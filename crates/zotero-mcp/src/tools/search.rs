use crate::core::reader::items::{get_item_by_key, hydrate_citation_key};
use crate::core::reader::search::{search_metadata, SearchParams};
use crate::core::reader::{collections, recent, tags};
use crate::core::types::Item;
use crate::state::AppState;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as Error;
use rmcp::Json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn map_err(e: crate::core::Error) -> Error {
    Error::internal_error(e.to_string(), None)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default = "default_true")]
    pub include_fulltext: bool,
    #[serde(default)]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}
fn default_true() -> bool {
    true
}

pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<CallToolResult, Error> {
    let hits = search_metadata(
        &s.pool,
        1,
        SearchParams {
            query: a.query,
            item_type: a.item_type,
            tag: a.tag,
            collection_key: a.collection,
            include_fulltext: a.include_fulltext,
            limit: a.limit,
            offset: a.offset,
        },
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&hits).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetItemArgs {
    #[serde(default)]
    pub item_key: Option<String>,
    #[serde(default)]
    pub citation_key: Option<String>,
}

pub async fn get_item(s: &AppState, a: GetItemArgs) -> Result<Json<Item>, Error> {
    let key = match (a.item_key, a.citation_key) {
        (Some(k), _) => k,
        (_, Some(_ck)) => {
            return Err(Error::invalid_params(
                "reverse citation_key lookup is not supported in v1; pass item_key",
                None,
            ));
        }
        _ => {
            return Err(Error::invalid_params(
                "either item_key or citation_key required",
                None,
            ))
        }
    };
    let mut item = get_item_by_key(&s.pool, &key, 1).await.map_err(map_err)?;
    hydrate_citation_key(&mut item, s.bbt.as_deref()).await;
    Ok(Json(item))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EmptyArgs {}

pub async fn list_collections(s: &AppState, _a: EmptyArgs) -> Result<CallToolResult, Error> {
    let cs = collections::list(&s.pool, 1, None).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&cs).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListTagsArgs {
    #[serde(default)]
    pub prefix: Option<String>,
}

pub async fn list_tags(s: &AppState, a: ListTagsArgs) -> Result<CallToolResult, Error> {
    let ts = tags::list(&s.pool, 1, a.prefix).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&ts).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RecentArgs {
    #[serde(default = "default_sort")]
    pub sort_by: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}
fn default_sort() -> String {
    "dateModified".into()
}
fn default_limit() -> i64 {
    20
}

pub async fn list_recent_items(s: &AppState, a: RecentArgs) -> Result<CallToolResult, Error> {
    let r = recent::list(&s.pool, 1, &a.sort_by, a.limit)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}
