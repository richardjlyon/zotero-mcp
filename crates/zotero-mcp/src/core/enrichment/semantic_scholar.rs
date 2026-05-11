use crate::core::cache::DiskCache;
use crate::core::error::{Error, Result};
use crate::core::enrichment::NormalizedRecord;
use crate::core::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct SemanticScholarClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
    api_key: Option<String>,
}

impl SemanticScholarClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http, api_key }
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<NormalizedRecord>> {
        let key = format!("ss:search:{}:{}", query, limit);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return Ok(parse(&v));
        }
        let url = format!("{}/graph/v1/paper/search", self.base);
        let fields = "title,year,abstract,externalIds,authors";
        let mut req = self.http.get(&url).query(&[
            ("query", query),
            ("limit", &limit.to_string()),
            ("fields", fields),
        ]);
        if let Some(k) = &self.api_key {
            req = req.header("x-api-key", k);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup {
                r#source: "semantic_scholar".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }
        let body: Value = resp.json().await?;
        self.cache.put(&key, &body).await.ok();
        Ok(parse(&body))
    }
}

fn parse(v: &Value) -> Vec<NormalizedRecord> {
    v.get("data")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let mut fields = Map::new();
                    if let Some(t) = p.get("title").and_then(|x| x.as_str()) {
                        fields.insert("title".into(), Value::String(t.into()));
                    }
                    if let Some(y) = p.get("year").and_then(|x| x.as_i64()) {
                        fields.insert("date".into(), Value::String(y.to_string()));
                    }
                    if let Some(a) = p.get("abstract").and_then(|x| x.as_str()) {
                        fields.insert("abstractNote".into(), Value::String(a.into()));
                    }
                    if let Some(doi) = p
                        .get("externalIds")
                        .and_then(|e| e.get("DOI"))
                        .and_then(|x| x.as_str())
                    {
                        fields.insert("DOI".into(), Value::String(doi.into()));
                    }
                    fields.insert("itemType".into(), Value::String("journalArticle".into()));

                    let creators = p
                        .get("authors")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
                                    let (first, last) = crate::core::enrichment::openlibrary_like_split(name);
                                    Creator {
                                        first_name: first,
                                        last_name: last,
                                        creator_type: "author".into(),
                                        order_index: i as i64,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    Some(NormalizedRecord {
                        source: "semantic_scholar".into(),
                        fields,
                        creators,
                        source_url: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
