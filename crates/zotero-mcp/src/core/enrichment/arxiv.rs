use crate::core::cache::DiskCache;
use crate::core::error::{Error, Result};
use crate::core::enrichment::NormalizedRecord;
use crate::core::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct ArxivClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl ArxivClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_arxiv(&self, id: &str) -> Result<NormalizedRecord> {
        let key = format!("arxiv:{}", id);
        if let Some(v) = self.cache.get::<String>(&key).await? {
            return parse_entry(&v).ok_or_else(|| Error::Lookup {
                r#source: "arxiv".into(),
                message: "cache parse failed".into(),
            });
        }
        let url = format!("{}/api/query?id_list={}", self.base, id);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup {
                r#source: "arxiv".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }
        let body = resp.text().await?;
        self.cache.put(&key, &body).await.ok();
        parse_entry(&body).ok_or_else(|| Error::Lookup {
            r#source: "arxiv".into(),
            message: "no entry".into(),
        })
    }
}

fn parse_entry(atom: &str) -> Option<NormalizedRecord> {
    // Minimal Atom-XML parser tailored to arXiv's known shape. For v1 we use a
    // simple scan rather than pulling a full XML parser; arXiv's format is
    // stable enough that this suffices.

    // Real arXiv feeds have a feed-level <title> before the entry <title>,
    // so we prefer the second occurrence. In tests/trimmed feeds there may
    // only be one, so fall back to the first occurrence.
    let title = atom
        .match_indices("<title>")
        .nth(1)
        .or_else(|| atom.match_indices("<title>").next())
        .and_then(|(i, _)| {
            let after = &atom[i + "<title>".len()..];
            after.find("</title>").map(|j| after[..j].trim().to_string())
        })?;

    let extract = |open: &str, close: &str| {
        let a = atom.find(open)?;
        let b = atom[a + open.len()..].find(close)?;
        Some(atom[a + open.len()..a + open.len() + b].trim().to_string())
    };
    let summary = extract("<summary>", "</summary>").unwrap_or_default();
    let published = extract("<published>", "</published>").unwrap_or_default();
    let date_only = published.split('T').next().unwrap_or(&published).to_string();

    let mut creators = vec![];
    let mut cursor = 0usize;
    let mut order = 0;
    while let Some(rel) = atom[cursor..].find("<author>") {
        let abs = cursor + rel + "<author>".len();
        if let Some(end) = atom[abs..].find("</author>") {
            let block = &atom[abs..abs + end];
            if let Some(nstart) = block.find("<name>") {
                let after = &block[nstart + "<name>".len()..];
                if let Some(nend) = after.find("</name>") {
                    let name = after[..nend].trim().to_string();
                    let (first, last) = super::openlibrary_like_split(&name);
                    creators.push(Creator {
                        first_name: first,
                        last_name: last,
                        creator_type: "author".into(),
                        order_index: order,
                    });
                    order += 1;
                }
            }
            cursor = abs + end + "</author>".len();
        } else {
            break;
        }
    }

    let mut fields = Map::new();
    fields.insert("title".into(), Value::String(title));
    if !summary.is_empty() {
        fields.insert("abstractNote".into(), Value::String(summary));
    }
    if !date_only.is_empty() {
        fields.insert("date".into(), Value::String(date_only));
    }
    fields.insert("itemType".into(), Value::String("preprint".into()));
    fields.insert("repository".into(), Value::String("arXiv".into()));

    Some(NormalizedRecord {
        source: "arxiv".into(),
        fields,
        creators,
        source_url: None,
    })
}
