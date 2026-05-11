use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::SearchHit;

#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub query: String,
    pub item_type: Option<String>,
    pub tag: Option<String>,
    pub collection_key: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn search_metadata(
    pool: &ReadOnlyPool,
    library_id: i64,
    mut params: SearchParams,
) -> Result<Vec<SearchHit>> {
    if params.limit <= 0 { params.limit = 50; }

    pool.with_conn(move |c| {
        let q = params.query.trim();
        let q_like = if q.is_empty() { "%".to_string() } else { format!("%{}%", q) };

        // Build base query. We resolve title/date via subqueries so the row stays one item.
        let mut sql = String::from(
            "SELECT DISTINCT i.itemID, i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')) AS title,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='date')) AS date
             FROM items i
             JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
             LEFT JOIN itemCreators ic ON ic.itemID = i.itemID
             LEFT JOIN creators cr ON cr.creatorID = ic.creatorID
             LEFT JOIN itemTags itag ON itag.itemID = i.itemID
             LEFT JOIN tags tg ON tg.tagID = itag.tagID
             LEFT JOIN collectionItems ci ON ci.itemID = i.itemID
             LEFT JOIN collections cl ON cl.collectionID = ci.collectionID
             WHERE i.libraryID = ?
               AND it.typeName NOT IN ('attachment','note','annotation')"
        );
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(library_id)];

        if !q.is_empty() {
            sql.push_str(" AND (
                EXISTS (SELECT 1 FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                        WHERE d.itemID = i.itemID AND v.value LIKE ?)
                OR cr.lastName LIKE ? OR cr.firstName LIKE ?
                OR tg.name LIKE ?
            )");
            for _ in 0..4 { binds.push(Box::new(q_like.clone())); }
        }

        if let Some(t) = &params.item_type {
            sql.push_str(" AND it.typeName = ?");
            binds.push(Box::new(t.clone()));
        }
        if let Some(t) = &params.tag {
            sql.push_str(" AND tg.name = ?");
            binds.push(Box::new(t.clone()));
        }
        if let Some(ck) = &params.collection_key {
            sql.push_str(" AND cl.key = ?");
            binds.push(Box::new(ck.clone()));
        }
        sql.push_str(" ORDER BY i.dateModified DESC LIMIT ? OFFSET ?");
        binds.push(Box::new(params.limit));
        binds.push(Box::new(params.offset));

        let params_refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| &**b).collect();
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query(params_refs.as_slice())?;
        let mut out = vec![];
        while let Some(r) = rows.next()? {
            let date: Option<String> = r.get(4)?;
            out.push(SearchHit {
                key: r.get(1)?,
                citation_key: None,
                item_type: r.get(2)?,
                title: r.get::<_, Option<String>>(3)?,
                creators_short: None,
                year: date.and_then(|s| s.split('-').next().map(str::to_string)),
                match_excerpt: None,
            });
        }
        Ok(out)
    }).await
}
