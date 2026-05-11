use crate::bbt::BbtClient;
use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::types::{Creator, Item};
use serde_json::{Map, Value};

pub async fn get_item_by_key(pool: &ReadOnlyPool, key: &str, library_id: i64) -> Result<Item> {
    let key_owned = key.to_string();
    let result: Option<Item> = pool
        .with_conn(move |c| {
            // Resolve itemID, itemType, base fields — returns None if missing.
            let row: rusqlite::Result<(i64, i64, String, String, i64)> = c.query_row(
                "SELECT i.itemID, i.itemTypeID, i.dateAdded, i.dateModified, i.version
                 FROM items i WHERE i.libraryID = ? AND i.key = ?",
                (library_id, &key_owned),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            );

            let (item_id, item_type_id, date_added, date_modified, version) = match row {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e),
            };

            let item_type: String = c.query_row(
                "SELECT typeName FROM itemTypes WHERE itemTypeID = ?",
                [item_type_id],
                |r| r.get(0),
            )?;

            // Fields
            let mut fields = Map::new();
            let mut stmt = c.prepare(
                "SELECT f.fieldName, v.value
                 FROM itemData d
                 JOIN fieldsCombined f ON f.fieldID = d.fieldID
                 JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = ?",
            )?;
            let mut rows = stmt.query([item_id])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(0)?;
                let value: String = r.get(1)?;
                fields.insert(name, Value::String(value));
            }

            // Creators
            let mut creators = vec![];
            let mut stmt = c.prepare(
                "SELECT cr.firstName, cr.lastName, ct.creatorType, ic.orderIndex
                 FROM itemCreators ic
                 JOIN creators cr ON cr.creatorID = ic.creatorID
                 JOIN creatorTypes ct ON ct.creatorTypeID = ic.creatorTypeID
                 WHERE ic.itemID = ?
                 ORDER BY ic.orderIndex ASC",
            )?;
            let mut rows = stmt.query([item_id])?;
            while let Some(r) = rows.next()? {
                creators.push(Creator {
                    first_name: r.get::<_, Option<String>>(0)?,
                    last_name: r.get::<_, Option<String>>(1)?,
                    creator_type: r.get::<_, String>(2)?,
                    order_index: r.get::<_, i64>(3)?,
                });
            }

            // Tags
            let mut tags = vec![];
            let mut stmt = c.prepare(
                "SELECT t.name FROM itemTags it \
                 JOIN tags t ON t.tagID = it.tagID \
                 WHERE it.itemID = ? ORDER BY t.name",
            )?;
            let mut rows = stmt.query([item_id])?;
            while let Some(r) = rows.next()? {
                tags.push(r.get::<_, String>(0)?);
            }

            // Collections
            let mut collection_keys = vec![];
            let mut stmt = c.prepare(
                "SELECT col.key FROM collectionItems ci \
                 JOIN collections col ON col.collectionID = ci.collectionID \
                 WHERE ci.itemID = ? ORDER BY ci.orderIndex",
            )?;
            let mut rows = stmt.query([item_id])?;
            while let Some(r) = rows.next()? {
                collection_keys.push(r.get::<_, String>(0)?);
            }

            // recommended_content_tool: child PDF → get_pdf_text;
            // HTML snapshot or `url` field → get_webpage_content; else none.
            let has_pdf: i64 = c.query_row(
                "SELECT COUNT(*) FROM itemAttachments a \
                 JOIN items i ON i.itemID = a.itemID \
                 WHERE a.parentItemID = ? AND a.contentType = 'application/pdf'",
                [item_id],
                |r| r.get(0),
            )?;
            let has_html: i64 = c.query_row(
                "SELECT COUNT(*) FROM itemAttachments a \
                 WHERE a.parentItemID = ? AND a.contentType = 'text/html'",
                [item_id],
                |r| r.get(0),
            )?;
            let has_url = fields.contains_key("url");
            let recommended_content_tool = if has_pdf > 0 {
                Some("get_pdf_text".to_string())
            } else if has_html > 0 || has_url {
                Some("get_webpage_content".to_string())
            } else {
                None
            };

            Ok(Some(Item {
                key: key_owned,
                library_id,
                version,
                item_type,
                citation_key: None, // populated later when BBT is wired in
                fields: Value::Object(fields),
                creators,
                tags,
                collection_keys,
                date_added,
                date_modified,
                parent_key: None,
                recommended_content_tool,
            }))
        })
        .await?;

    result.ok_or_else(|| Error::ItemNotFound(key.to_string()))
}

pub async fn hydrate_citation_key(item: &mut Item, bbt: Option<&BbtClient>) {
    if item.citation_key.is_some() { return; }
    let Some(client) = bbt else { return };
    if let Ok(map) = client.citationkeys(&[item.key.clone()]).await {
        if let Some(ck) = map.get(&item.key) {
            item.citation_key = Some(ck.clone());
        }
    }
}
