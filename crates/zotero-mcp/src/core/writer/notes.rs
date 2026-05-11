use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use serde_json::{json, Value};

/// Adds a child note to a parent item. Markdown is wrapped in <p>; Zotero's
/// note storage accepts HTML, so we convert via a minimal markdown-to-HTML
/// pass (paragraphs, headings, emphasis). For richer formatting, callers can
/// pass HTML directly — it will pass through if it starts with `<`.
pub async fn add_note(api: &LocalApi, parent_key: &str, markdown_or_html: &str) -> Result<String> {
    let html = if markdown_or_html.trim_start().starts_with('<') {
        markdown_or_html.to_string()
    } else {
        markdown_to_simple_html(markdown_or_html)
    };

    let body = json!([{
        "itemType": "note",
        "parentItem": parent_key,
        "note": html,
        "tags": [],
        "relations": {}
    }]);
    let url = api.user_path("/items");
    let resp = api.http.post(&url)
        .header("Zotero-API-Version", "3")
        .json(&body)
        .send().await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi { status: status.as_u16(), body: v.to_string() });
    }
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi { status: 200, body: v.to_string() })
}

fn markdown_to_simple_html(md: &str) -> String {
    // Tiny, conservative converter: H1-H3, bold, italic, paragraphs.
    let mut out = String::new();
    for para in md.split("\n\n") {
        let p = para.trim();
        if p.is_empty() { continue; }
        if let Some(rest) = p.strip_prefix("### ") {
            out.push_str(&format!("<h3>{}</h3>", html_escape(rest)));
        } else if let Some(rest) = p.strip_prefix("## ") {
            out.push_str(&format!("<h2>{}</h2>", html_escape(rest)));
        } else if let Some(rest) = p.strip_prefix("# ") {
            out.push_str(&format!("<h1>{}</h1>", html_escape(rest)));
        } else {
            out.push_str(&format!("<p>{}</p>", inline(&html_escape(p))));
        }
    }
    out
}

fn inline(s: &str) -> String {
    // **bold** -> <strong>; *italic* -> <em>
    // ORDER MATTERS: replace ** before * so that "*" inside "**...**" isn't grabbed
    let s = regex_lite_replace(s, "**", "<strong>", "</strong>");
    regex_lite_replace(&s, "*", "<em>", "</em>")
}

fn regex_lite_replace(s: &str, delim: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find(delim) {
            None => { out.push_str(rest); return out; }
            Some(a) => {
                out.push_str(&rest[..a]);
                let after = &rest[a + delim.len()..];
                match after.find(delim) {
                    Some(b) => {
                        out.push_str(open);
                        out.push_str(&after[..b]);
                        out.push_str(close);
                        rest = &after[b + delim.len()..];
                    }
                    None => { out.push_str(delim); out.push_str(after); return out; }
                }
            }
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
