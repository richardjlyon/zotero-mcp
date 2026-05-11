use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct CrossrefClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl CrossrefClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_doi(&self, doi: &str) -> Result<NormalizedRecord> {
        let key = format!("crossref:doi:{}", doi);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return normalize_work(&v).ok_or_else(|| Error::Lookup {
                r#source: "crossref".into(),
                message: "cache parse failed".into(),
            });
        }
        let url = format!("{}/works/{}", self.base, doi);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup {
                r#source: "crossref".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }
        let body: Value = resp.json().await?;
        let msg = body.get("message").cloned().unwrap_or_default();
        self.cache.put(&key, &msg).await.ok();
        normalize_work(&msg).ok_or_else(|| Error::Lookup {
            r#source: "crossref".into(),
            message: "no fields".into(),
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<NormalizedRecord>> {
        let key = format!("crossref:search:{}:{}", query, limit);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return Ok(parse_search(&v));
        }
        let url = format!("{}/works", self.base);
        let resp = self.http.get(&url)
            .query(&[("query", query), ("rows", &limit.to_string())])
            .send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup {
                r#source: "crossref".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }
        let body: Value = resp.json().await?;
        self.cache.put(&key, &body).await.ok();
        Ok(parse_search(&body))
    }
}

fn parse_search(v: &Value) -> Vec<NormalizedRecord> {
    v.get("message")
        .and_then(|m| m.get("items"))
        .and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(normalize_work).collect())
        .unwrap_or_default()
}

fn normalize_work(msg: &Value) -> Option<NormalizedRecord> {
    let mut fields = Map::new();
    if let Some(t) = msg.get("title").and_then(|x| x.as_array()).and_then(|a| a.first()) {
        if let Some(s) = t.as_str() {
            fields.insert("title".into(), Value::String(s.to_string()));
        }
    }
    if let Some(doi) = msg.get("DOI").and_then(|x| x.as_str()) {
        fields.insert("DOI".into(), Value::String(doi.to_string()));
    }
    if let Some(date) = extract_date(msg) {
        fields.insert("date".into(), Value::String(date));
    }
    if let Some(c) = msg.get("container-title")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
    {
        fields.insert("publicationTitle".into(), Value::String(c.into()));
    }
    if let Some(pub_) = msg.get("publisher").and_then(|x| x.as_str()) {
        fields.insert("publisher".into(), Value::String(pub_.into()));
    }
    if let Some(url) = msg.get("URL").and_then(|x| x.as_str()) {
        fields.insert("url".into(), Value::String(url.into()));
    }
    if let Some(abs) = msg.get("abstract").and_then(|x| x.as_str()) {
        fields.insert("abstractNote".into(), Value::String(strip_html(abs)));
    }
    let item_type = match msg.get("type").and_then(|x| x.as_str()) {
        Some("journal-article") => "journalArticle",
        Some("book") => "book",
        Some("book-chapter") => "bookSection",
        Some("proceedings-article") => "conferencePaper",
        Some("posted-content") => "preprint",
        _ => "document",
    };
    fields.insert("itemType".into(), Value::String(item_type.into()));

    let creators = msg
        .get("author")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, a)| Creator {
                    first_name: a.get("given").and_then(|x| x.as_str()).map(String::from),
                    last_name: a.get("family").and_then(|x| x.as_str()).map(String::from),
                    creator_type: "author".into(),
                    order_index: i as i64,
                })
                .collect()
        })
        .unwrap_or_default();

    Some(NormalizedRecord {
        source: "crossref".into(),
        fields,
        creators,
        source_url: msg.get("URL").and_then(|x| x.as_str()).map(String::from),
    })
}

fn extract_date(msg: &Value) -> Option<String> {
    let parts = msg
        .get("issued")
        .or_else(|| msg.get("published-print"))
        .or_else(|| msg.get("published-online"))?;
    let arr = parts.get("date-parts")?.as_array()?.first()?.as_array()?;
    let nums: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_i64().map(|n| format!("{:02}", n)))
        .collect();
    if nums.is_empty() {
        return None;
    }
    Some(match nums.len() {
        1 => nums[0].clone(),
        2 => format!("{}-{}", nums[0], nums[1]),
        _ => format!("{}-{}-{}", nums[0], nums[1], nums[2]),
    })
}

fn strip_html(s: &str) -> String {
    // Lightweight tag stripper for CrossRef's JATS-flavored abstracts.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}
