//! Resilience tests for the three `lookup_*` paths.
//!
//! Every test asserts on the *number of requests the server received*, so a
//! missing retry or an unwanted one fails loudly rather than passing by luck.
//! The ISBN pair used throughout (9781844674879 / 1844674878) is the one from
//! the 2026-05-13 OpenLibrary failure.

use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::arxiv::ArxivClient;
use zotero_mcp::core::enrichment::crossref::CrossrefClient;
use zotero_mcp::core::enrichment::openlibrary::OpenLibraryClient;
use zotero_mcp::core::error::Error;

const ISBN13: &str = "9781844674879";
const ISBN10: &str = "1844674878";

fn cache() -> (tempfile::TempDir, DiskCache) {
    let dir = tempdir().unwrap();
    let c = DiskCache::new(dir.path().to_path_buf(), 60);
    (dir, c)
}

fn book_body() -> serde_json::Value {
    serde_json::json!({
        "title": "What is Modern Israel?",
        "publish_date": "2016",
        "publishers": ["Pluto Press"]
    })
}

fn failure(e: Error) -> zotero_mcp::core::enrichment::resilience::LookupFailure {
    match e {
        Error::LookupFailed(f) => *f,
        other => panic!("expected a structured lookup failure, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// OpenLibrary — the 2026-05-13 outage path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openlibrary_transient_failure_retries_once_then_succeeds() {
    let server = MockServer::start().await;
    // First call 503, second 200 — wiremock serves mounts in order, so an
    // up-to-1-time 503 followed by the real body models a blip.
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{ISBN13}.json")))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{ISBN13}.json")))
        .respond_with(ResponseTemplate::new(200).set_body_json(book_body()))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = OpenLibraryClient::new(server.uri(), cache, "test/0.1");
    let r = c.lookup_isbn(ISBN13).await.expect("retry should succeed");
    assert_eq!(r.fields["title"], "What is Modern Israel?");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "expected exactly one retry"
    );
}

#[tokio::test]
async fn openlibrary_404_tries_alternate_isbn_form() {
    let server = MockServer::start().await;
    // The ISBN-13 is not indexed; the ISBN-10 is. This is the case a caller
    // holding the number printed on an older paperback hits.
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{ISBN13}.json")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{ISBN10}.json")))
        .respond_with(ResponseTemplate::new(200).set_body_json(book_body()))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = OpenLibraryClient::new(server.uri(), cache, "test/0.1");
    let r = c
        .lookup_isbn(ISBN13)
        .await
        .expect("alternate form should be tried");
    assert_eq!(r.fields["title"], "What is Modern Israel?");
    // One 404 (not retried — permanent) plus one success on the other form.
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn openlibrary_hyphenated_input_is_normalised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{ISBN13}.json")))
        .respond_with(ResponseTemplate::new(200).set_body_json(book_body()))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = OpenLibraryClient::new(server.uri(), cache, "test/0.1");
    let r = c.lookup_isbn("978-1-84467-487-9").await.unwrap();
    assert_eq!(r.fields["title"], "What is Modern Israel?");
}

#[tokio::test]
async fn openlibrary_total_failure_returns_structured_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = OpenLibraryClient::new(server.uri(), cache, "test/0.1");
    let f = failure(c.lookup_isbn(ISBN13).await.unwrap_err());

    assert_eq!(f.error, "lookup_failed");
    assert_eq!(f.source, "openlibrary");
    assert_eq!(f.identifier, ISBN13);
    // Two forms × (attempt + one retry) = 4 recorded attempts.
    assert_eq!(f.attempts.len(), 4, "attempts: {:?}", f.attempts);
    assert!(f.attempts.iter().all(|a| a.transient));
    assert!(f.attempts.iter().any(|a| a.identifier == ISBN10));
    assert_eq!(f.suggestion, "fall_back_to_hand_built");
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn openlibrary_forbidden_suggests_stopping_to_ask() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = OpenLibraryClient::new(server.uri(), cache, "test/0.1");
    let f = failure(c.lookup_isbn(ISBN13).await.unwrap_err());
    assert_eq!(
        f.suggestion, "stop_and_ask",
        "a 403 is not 'the catalogue lacks it' — hand-building would paper over an access problem"
    );
    // 403 is permanent: one attempt per form, no retries.
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// CrossRef
// ---------------------------------------------------------------------------

#[tokio::test]
async fn crossref_404_does_not_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = CrossrefClient::new(server.uri(), cache, "test/0.1");
    let f = failure(c.lookup_doi("10.1234/abcd").await.unwrap_err());
    assert_eq!(f.attempts.len(), 1, "a 404 means CrossRef lacks the DOI");
    assert!(!f.attempts[0].transient);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn crossref_normalises_doi_url_form() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "message": { "title": ["A Paper"], "type": "journal-article" }
        })))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = CrossrefClient::new(server.uri(), cache, "test/0.1");
    let r = c
        .lookup_doi("https://doi.org/10.1234/abcd")
        .await
        .expect("a URL-form DOI must work");
    assert_eq!(r.fields["title"], "A Paper");
}

#[tokio::test]
async fn crossref_retries_transient_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "message": { "title": ["A Paper"], "type": "journal-article" }
        })))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = CrossrefClient::new(server.uri(), cache, "test/0.1");
    let r = c.lookup_doi("10.1234/abcd").await.unwrap();
    assert_eq!(r.fields["title"], "A Paper");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// arXiv
// ---------------------------------------------------------------------------

fn atom(title: &str) -> String {
    format!(
        r#"<feed xmlns="http://www.w3.org/2005/Atom">
             <title>arXiv Query</title>
             <entry>
               <title>{title}</title>
               <published>2024-01-15T00:00:00Z</published>
               <summary>An abstract.</summary>
               <author><name>Jane Doe</name></author>
             </entry>
           </feed>"#
    )
}

#[tokio::test]
async fn arxiv_retries_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/query"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(atom("A Preprint")))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = ArxivClient::new(server.uri(), cache, "test/0.1");
    let r = c.lookup_arxiv("2401.12345").await.unwrap();
    assert_eq!(r.fields["title"], "A Preprint");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn arxiv_normalises_prefixed_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(atom("A Preprint")))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = ArxivClient::new(server.uri(), cache, "test/0.1");
    c.lookup_arxiv("arXiv:2401.12345")
        .await
        .expect("a prefixed id must work");
    let reqs = server.received_requests().await.unwrap();
    let url = reqs[0].url.to_string();
    assert!(
        url.contains("id_list=2401.12345") && !url.to_lowercase().contains("arxiv%3a"),
        "prefix should have been stripped before the request: {url}"
    );
}

#[tokio::test]
async fn arxiv_total_failure_is_structured() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let (_d, cache) = cache();
    let c = ArxivClient::new(server.uri(), cache, "test/0.1");
    let f = failure(c.lookup_arxiv("2401.12345").await.unwrap_err());
    assert_eq!(f.source, "arxiv");
    assert_eq!(f.attempts.len(), 2, "one attempt plus one retry");
    assert_eq!(f.suggestion, "fall_back_to_hand_built");
}
