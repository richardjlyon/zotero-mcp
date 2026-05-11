use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::Error;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zotero_core::reader::items::get_item_by_key;
use zotero_core::writer::items::update_item_fields;
use zotero_core::writer::notes::add_note;
use zotero_core::writer::tags::{add_tags, add_to_collection, remove_from_collection, remove_tags};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddNoteArgs {
    pub item_key: String,
    pub markdown: String,
}

pub async fn add_note_t(s: &AppState, a: AddNoteArgs) -> Result<CallToolResult, Error> {
    let k = add_note(&s.api, &a.item_key, &a.markdown).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text(k)]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateFieldsArgs {
    pub item_key: String,
    pub fields: std::collections::BTreeMap<String, serde_json::Value>,
}

pub async fn update_item_fields_t(s: &AppState, a: UpdateFieldsArgs) -> Result<CallToolResult, Error> {
    let item = get_item_by_key(&s.pool, &a.item_key, 1).await.map_err(map_err)?;
    update_item_fields(&s.api, &a.item_key, item.version, serde_json::Value::Object(
        a.fields.into_iter().collect()
    ))
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TagArgs {
    pub item_key: String,
    pub tags: Vec<String>,
}

pub async fn add_tags_t(s: &AppState, a: TagArgs) -> Result<CallToolResult, Error> {
    add_tags(&s.api, &a.item_key, &a.tags).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

pub async fn remove_tags_t(s: &AppState, a: TagArgs) -> Result<CallToolResult, Error> {
    remove_tags(&s.api, &a.item_key, &a.tags).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CollectionArgs {
    pub item_key: String,
    pub collection_key: String,
}

pub async fn add_to_collection_t(s: &AppState, a: CollectionArgs) -> Result<CallToolResult, Error> {
    add_to_collection(&s.api, &a.item_key, &a.collection_key).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

pub async fn remove_from_collection_t(s: &AppState, a: CollectionArgs) -> Result<CallToolResult, Error> {
    remove_from_collection(&s.api, &a.item_key, &a.collection_key).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
