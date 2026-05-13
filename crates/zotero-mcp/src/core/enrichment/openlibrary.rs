use crate::core::cache::DiskCache;
use crate::core::error::{Error, Result};
use crate::core::enrichment::NormalizedRecord;
use crate::core::types::Creator;
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

const MONTHS: &[(&str, u32)] = &[
    ("january", 1), ("jan", 1),
    ("february", 2), ("feb", 2),
    ("march", 3), ("mar", 3),
    ("april", 4), ("apr", 4),
    ("may", 5),
    ("june", 6), ("jun", 6),
    ("july", 7), ("jul", 7),
    ("august", 8), ("aug", 8),
    ("september", 9), ("sept", 9), ("sep", 9),
    ("october", 10), ("oct", 10),
    ("november", 11), ("nov", 11),
    ("december", 12), ("dec", 12),
];

/// Parse OpenLibrary's freeform `publish_date` into ISO 8601 (YYYY-MM-DD,
/// YYYY-MM, or YYYY). Returns the trimmed input unchanged if the string
/// doesn't cleanly match a known pattern — never drops information.
pub(crate) fn parse_date(s: &str) -> String {
    let trimmed = s.trim();
    if is_iso_date(trimmed) {
        return trimmed.to_string();
    }

    let cleaned = trimmed.to_lowercase().replace(',', " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();

    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut year: Option<u32> = None;
    let mut unrecognised = 0;

    for tok in &tokens {
        if let Some((_, m)) = MONTHS.iter().find(|(name, _)| *name == *tok) {
            if month.is_none() {
                month = Some(*m);
            }
        } else if let Ok(n) = tok.parse::<u32>() {
            if (1000..=9999).contains(&n) {
                year = Some(n);
            } else if (1..=31).contains(&n) {
                day = Some(n);
            } else {
                unrecognised += 1;
            }
        } else {
            unrecognised += 1;
        }
    }

    if unrecognised > 0 {
        return trimmed.to_string();
    }

    match (year, month, day) {
        (Some(y), Some(m), Some(d)) => format!("{:04}-{:02}-{:02}", y, m, d),
        (Some(y), Some(m), None) => format!("{:04}-{:02}", y, m),
        (Some(y), None, None) => format!("{:04}", y),
        _ => trimmed.to_string(),
    }
}

fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.iter().any(|p| !p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    match parts.as_slice() {
        [y] if y.len() == 4 => true,
        [y, m] if y.len() == 4 && m.len() == 2 => true,
        [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => true,
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::parse_date;

    #[test]
    fn parse_date_iso_year_passes_through() {
        assert_eq!(parse_date("2020"), "2020");
    }

    #[test]
    fn parse_date_iso_year_month_passes_through() {
        assert_eq!(parse_date("2020-05"), "2020-05");
    }

    #[test]
    fn parse_date_iso_full_passes_through() {
        assert_eq!(parse_date("1998-09-08"), "1998-09-08");
    }

    #[test]
    fn parse_date_long_month_day_year() {
        assert_eq!(parse_date("March 5, 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_short_month_day_year() {
        assert_eq!(parse_date("Mar 5, 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_day_long_month_year() {
        assert_eq!(parse_date("5 March 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_long_month_year() {
        assert_eq!(parse_date("March 2020"), "2020-03");
    }

    #[test]
    fn parse_date_short_month_year() {
        assert_eq!(parse_date("Mar 2020"), "2020-03");
    }

    #[test]
    fn parse_date_unparseable_passes_through() {
        assert_eq!(parse_date("sometime in 2020"), "sometime in 2020");
    }

    #[test]
    fn parse_date_trims_whitespace() {
        assert_eq!(parse_date("  2020  "), "2020");
    }
}
