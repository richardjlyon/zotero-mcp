use crate::core::dedup::{self, DedupInput, FindDuplicatesResult, InputKind};
use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::ErrorData as Error;
use rmcp::Json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FindDuplicatesArgs {
    /// The work's title, or the raw filename it arrived as — the tool strips
    /// extensions, copy counters and shadow-library noise itself, so pass what
    /// you have rather than cleaning it up first.
    #[serde(default)]
    pub title: Option<String>,
    /// Surname of the first-named author. Worth supplying even when you have a
    /// title: the author pass catches records whose title or first name is
    /// recorded differently from yours.
    #[serde(default)]
    pub author_surname: Option<String>,
    /// A DOI, ISBN or arXiv id in any form — URL-wrapped, prefixed, hyphenated.
    /// Both ISBN-10 and ISBN-13 forms are searched.
    #[serde(default)]
    pub identifier: Option<String>,
    /// What you are holding: "pdf" (a file to attach), "url" (a page to
    /// snapshot), or "name" (bibliographic details only). Decides whether an
    /// existing record counts as complete or as wanting your file.
    #[serde(default)]
    pub input_kind: InputKind,
    /// Rows per search pass. Omit for the default (200).
    #[serde(default)]
    pub limit: i64,
}

pub async fn find_duplicates_t(
    s: &AppState,
    a: FindDuplicatesArgs,
) -> Result<Json<FindDuplicatesResult>, Error> {
    let input = DedupInput {
        title: a.title,
        author_surname: a.author_surname,
        identifier: a.identifier,
        input_kind: a.input_kind,
        limit: a.limit,
    };
    // Reject a query-less call before touching the library: with nothing to
    // search on, every item would qualify as a "candidate".
    input
        .validate()
        .map_err(|m| Error::invalid_params(m, None))?;

    let storage_dir = s.cfg.storage_dir();
    let r = dedup::find_duplicates(&s.pool, 1, &storage_dir, &input)
        .await
        .map_err(map_err)?;
    Ok(Json(r))
}
