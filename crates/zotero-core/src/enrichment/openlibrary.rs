use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct OpenLibraryClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl OpenLibraryClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_isbn(&self, isbn: &str) -> Result<NormalizedRecord> {
        let key = format!("openlibrary:isbn:{}", isbn);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return self.from_book_json(v).await;
        }
        let url = format!("{}/isbn/{}.json", self.base, isbn);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup {
                r#source: "openlibrary".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }
        let book: Value = resp.json().await?;
        self.cache.put(&key, &book).await.ok();
        self.from_book_json(book).await
    }

    async fn from_book_json(&self, book: Value) -> Result<NormalizedRecord> {
        let mut fields = Map::new();
        if let Some(t) = book.get("title").and_then(|x| x.as_str()) {
            fields.insert("title".into(), Value::String(t.into()));
        }
        if let Some(d) = book.get("publish_date").and_then(|x| x.as_str()) {
            fields.insert("date".into(), Value::String(d.into()));
        }
        if let Some(p) = book
            .get("publishers")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
        {
            fields.insert("publisher".into(), Value::String(p.into()));
        }
        fields.insert("itemType".into(), Value::String("book".into()));

        let mut creators = vec![];
        if let Some(authors) = book.get("authors").and_then(|x| x.as_array()) {
            for (i, a) in authors.iter().enumerate() {
                if let Some(akey) = a.get("key").and_then(|x| x.as_str()) {
                    let name = self
                        .resolve_author_name(akey)
                        .await
                        .unwrap_or_else(|| "Unknown".into());
                    let (first, last) = split_name(&name);
                    creators.push(Creator {
                        first_name: first,
                        last_name: last,
                        creator_type: "author".into(),
                        order_index: i as i64,
                    });
                }
            }
        }

        Ok(NormalizedRecord {
            source: "openlibrary".into(),
            fields,
            creators,
            source_url: Some(format!("{}{}", self.base, "/")),
        })
    }

    async fn resolve_author_name(&self, key: &str) -> Option<String> {
        let cache_key = format!("openlibrary:author:{}", key);
        if let Ok(Some(v)) = self.cache.get::<Value>(&cache_key).await {
            return v.get("name").and_then(|x| x.as_str()).map(String::from);
        }
        let url = format!("{}{}.json", self.base, key);
        let resp = self.http.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let v: Value = resp.json().await.ok()?;
        self.cache.put(&cache_key, &v).await.ok();
        v.get("name").and_then(|x| x.as_str()).map(String::from)
    }
}

fn split_name(full: &str) -> (Option<String>, Option<String>) {
    // Naive: last token is surname; everything before is first.
    let parts: Vec<&str> = full.trim().rsplitn(2, ' ').collect();
    match parts.as_slice() {
        [last, first] => (Some((*first).to_string()), Some((*last).to_string())),
        [single] => (None, Some((*single).to_string())),
        _ => (None, None),
    }
}
