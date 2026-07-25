use crate::core::reader::items::{get_item_by_key, hydrate_citation_key};
use crate::core::reader::search::{search_metadata, SearchParams, DEFAULT_SEARCH_LIMIT};
use crate::core::reader::{collections, recent, tags};
use crate::core::types::{Collection, Item, SearchHit, Tag};
use crate::state::AppState;
use crate::tools::wire::ListResult;
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

pub async fn search_items(
    s: &AppState,
    a: SearchArgs,
) -> Result<Json<ListResult<SearchHit>>, Error> {
    // The core substitutes its default for a non-positive limit; mirror that
    // here so the truncation flag reflects the limit actually applied.
    let effective_limit = if a.limit > 0 {
        a.limit
    } else {
        DEFAULT_SEARCH_LIMIT
    };
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
    Ok(Json(ListResult::with_limit(hits, effective_limit)))
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

pub async fn list_collections(
    s: &AppState,
    _a: EmptyArgs,
) -> Result<Json<ListResult<Collection>>, Error> {
    let cs = collections::list(&s.pool, 1, None).await.map_err(map_err)?;
    Ok(Json(ListResult::complete(cs)))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListTagsArgs {
    #[serde(default)]
    pub prefix: Option<String>,
}

pub async fn list_tags(s: &AppState, a: ListTagsArgs) -> Result<Json<ListResult<Tag>>, Error> {
    let ts = tags::list(&s.pool, 1, a.prefix).await.map_err(map_err)?;
    Ok(Json(ListResult::complete(ts)))
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

pub async fn list_recent_items(
    s: &AppState,
    a: RecentArgs,
) -> Result<Json<ListResult<SearchHit>>, Error> {
    let limit = a.limit;
    let r = recent::list(&s.pool, 1, &a.sort_by, a.limit)
        .await
        .map_err(map_err)?;
    Ok(Json(ListResult::with_limit(r, limit)))
}
