use std::sync::Arc;
use zotero_core::bbt::BbtClient;
use zotero_core::cache::DiskCache;
use zotero_core::config::Config;
use zotero_core::enrichment::arxiv::ArxivClient;
use zotero_core::enrichment::crossref::CrossrefClient;
use zotero_core::enrichment::openlibrary::OpenLibraryClient;
use zotero_core::enrichment::semantic_scholar::SemanticScholarClient;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::writer::client::LocalApi;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub pool: ReadOnlyPool,
    pub api: LocalApi,
    pub bbt: Option<Arc<BbtClient>>,
    pub crossref: CrossrefClient,
    pub openlibrary: OpenLibraryClient,
    pub arxiv: ArxivClient,
    pub semantic_scholar: SemanticScholarClient,
}

impl AppState {
    pub async fn build(cfg: Config) -> anyhow::Result<Self> {
        let pool = ReadOnlyPool::new(cfg.sqlite_path(), 4).await?;
        // Schema-version check at startup
        {
            let conn = zotero_core::reader::conn::open_read_only(&cfg.sqlite_path())?;
            zotero_core::reader::conn::check_schema(
                &conn,
                cfg.zotero.min_schema_userdata,
                cfg.zotero.max_schema_userdata,
            )?;
        }
        let user_id = if cfg.zotero.user_id > 0 {
            cfg.zotero.user_id
        } else {
            detect_user_id(&pool, &cfg.zotero.local_api_base).await?
        };
        let api = LocalApi::new(cfg.zotero.local_api_base.clone(), user_id)?;
        let bbt = BbtClient::new(cfg.zotero.local_api_base.clone())
            .ok()
            .map(Arc::new);

        let cache = DiskCache::new(
            cfg.resolved_cache_dir(),
            cfg.enrichment.cache_ttl_days * 86_400,
        );
        let ua = cfg.web.user_agent.clone();
        let crossref = CrossrefClient::new("https://api.crossref.org", cache.clone(), &ua);
        let openlibrary = OpenLibraryClient::new("https://openlibrary.org", cache.clone(), &ua);
        let arxiv = ArxivClient::new("https://export.arxiv.org", cache.clone(), &ua);
        let semantic_scholar =
            SemanticScholarClient::new("https://api.semanticscholar.org", cache, &ua, None);

        Ok(Self {
            cfg,
            pool,
            api,
            bbt,
            crossref,
            openlibrary,
            arxiv,
            semantic_scholar,
        })
    }
}

async fn detect_user_id(pool: &ReadOnlyPool, base: &str) -> anyhow::Result<i64> {
    // First try reading the userID directly from Zotero's SQLite — this works
    // whether Zotero is running or not and doesn't depend on the Local API
    // exposing a `whoami` endpoint.
    let from_db = pool
        .with_conn(|c| c.query_row("SELECT userID FROM users LIMIT 1", [], |r| r.get::<_, i64>(0)))
        .await
        .ok();
    if let Some(id) = from_db {
        return Ok(id);
    }

    // Fallback: probe the Local API. Zotero does not document a guaranteed
    // whoami endpoint, but `/api/keys/current` returns the userID when an
    // API-key context is present.
    let resp = reqwest::Client::new()
        .get(format!("{}/api/keys/current", base))
        .header("Zotero-API-Version", "3")
        .send()
        .await?;
    if resp.status().is_success() {
        let v: serde_json::Value = resp.json().await?;
        if let Some(id) = v.get("userID").and_then(|x| x.as_i64()) {
            return Ok(id);
        }
    }
    Err(anyhow::anyhow!(
        "could not auto-detect Zotero user_id; set zotero.user_id in config"
    ))
}
