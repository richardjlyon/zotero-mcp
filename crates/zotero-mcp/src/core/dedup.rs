//! The composite dedup gate behind the `find_duplicates` tool.
//!
//! Three passes over the local library — title tokens, author surname,
//! identifier — unioned, trash-filtered, and triaged. Each pass exists because
//! a single-pass check demonstrably failed on 2026-05-13 (see
//! `docs/superpowers/specs/2026-07-25-find-duplicates-design.md`):
//!
//! - the **title** pass matches on individual significant words rather than the
//!   whole title, because `search_metadata` is one SQL `LIKE '%query%'` and a
//!   single punctuation difference on the *stored* side defeats it
//!   (`Gaza: An inquest…` versus a query of `Gaza An Inquest…`);
//! - the **author** pass exists because a title pass alone missed a record whose
//!   author's first name was misspelt ("Yakob" for "Yakov"), and a surname
//!   search does not care about first names;
//! - the author pass is then **title-filtered**, because a surname search
//!   otherwise returns everything that author ever wrote.

use crate::core::error::{Error, Result};
use crate::core::isbn;
use crate::core::reader::attachments::list_attachments;
use crate::core::reader::items::get_item_by_key;
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::search::{search_metadata, SearchParams};
use crate::core::reader::trash::trashed_keys;
use crate::core::types::{AttachmentLinkMode, Item, SearchHit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Minimum significant words an input title and a candidate title must share
/// for the candidate to survive the title pass. Two is a deliberately low bar:
/// this is a gate, and a spurious candidate costs the caller one glance at
/// `title_similarity`, while a missed duplicate costs a polluted library.
const MIN_SHARED_TOKENS: usize = 2;

/// Below this token-set overlap a candidate is "near match, ask the human"
/// rather than "same work".
const WEAK_SIMILARITY: f64 = 0.5;

/// Rows per SQL pass. Generous because a token like "modern" is not selective.
const DEFAULT_LIMIT: i64 = 200;

/// How many title words to query on. Each is one `LIKE` scan against a local
/// SQLite file, so the ceiling is about restraint, not cost. Three proved too
/// few: an input of "Modern Israel Studies Handbook Companion" spends its three
/// longest words on tokens the target record does not have, and misses a record
/// titled "What is Modern Israel?" whose only shared words are the short ones.
const MAX_TITLE_QUERIES: usize = 6;

/// Words carrying no discriminating power in a title. Kept short on purpose —
/// the length ≥ 4 rule in [`significant_tokens`] already removes most noise.
const STOP_WORDS: &[&str] = &[
    "about", "after", "again", "against", "along", "also", "among", "and", "another", "any", "are",
    "around", "because", "been", "before", "being", "between", "both", "but", "does", "down",
    "during", "each", "either", "for", "from", "further", "have", "here", "how", "however", "into",
    "its", "itself", "just", "more", "most", "much", "must", "neither", "nor", "not", "onto",
    "other", "our", "out", "over", "own", "same", "should", "since", "some", "such", "than",
    "that", "the", "their", "them", "then", "there", "these", "they", "this", "those", "through",
    "thus", "toward", "towards", "under", "until", "upon", "very", "was", "were", "what", "when",
    "where", "which", "while", "who", "whom", "whose", "why", "will", "with", "within", "without",
    "would", "your",
];

const EBOOK_EXTENSIONS: &[&str] = &[".pdf", ".epub", ".mobi", ".azw3", ".djvu", ".txt"];

/// Fields whose absence on an existing record is worth telling the caller
/// about, so it can offer what it has.
const SPARSENESS_FIELDS: &[&str] = &[
    "ISBN",
    "DOI",
    "publisher",
    "place",
    "date",
    "abstractNote",
    "language",
    "numPages",
    "url",
];

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// What kind of thing the caller is holding. Drives the attachment comparison:
/// a PDF in hand is only a duplicate of a record that already *has* a PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// A file on disk to attach.
    Pdf,
    /// A web page to snapshot or link.
    Url,
    /// Bibliographic details only, nothing to attach.
    #[default]
    Name,
}

#[derive(Debug, Clone, Default)]
pub struct DedupInput {
    pub title: Option<String>,
    pub author_surname: Option<String>,
    pub identifier: Option<String>,
    pub input_kind: InputKind,
    /// Rows per SQL pass; 0 or negative means the default (200).
    pub limit: i64,
}

impl DedupInput {
    /// At least one of title / author_surname / identifier must be usable —
    /// otherwise there is no query to run and every item in the library would
    /// be a "candidate".
    pub fn validate(&self) -> std::result::Result<(), String> {
        let usable = |o: &Option<String>| o.as_deref().map(|s| !s.trim().is_empty()) == Some(true);
        if usable(&self.title) || usable(&self.author_surname) || usable(&self.identifier) {
            Ok(())
        } else {
            Err("at least one of title, author_surname or identifier is required".into())
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Triage {
    /// The record already has the same kind of attachment as the input.
    #[serde(rename = "i")]
    I,
    /// The record exists but lacks what the caller is holding.
    #[serde(rename = "ii")]
    Ii,
    /// Near match — titles disagree, or identifiers conflict.
    #[serde(rename = "iii")]
    Iii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Already in the library, complete. Don't add anything.
    Abort,
    /// The record is there but wants the caller's file or snapshot.
    AttachToExisting,
    /// Put it to the user — the tool is not confident.
    Ask,
    /// Nothing matched; safe to create.
    CreateNew,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryRun {
    /// Which pass ran this query: `title`, `author`, or `identifier`.
    pub pass: String,
    pub query: String,
    /// Rows the library returned.
    pub result_count: usize,
    /// Rows that survived this pass's filter.
    pub kept: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CandidateAttachment {
    pub link_mode: AttachmentLinkMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataDiff {
    /// Fields the existing record does not have at all.
    pub missing: Vec<String>,
    /// Fields present but plainly thin (e.g. a year-only date).
    pub thin: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DuplicateCandidate {
    pub item_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creators_short: Option<String>,
    pub item_type: String,
    /// Which passes surfaced this candidate.
    pub found_by: Vec<String>,
    /// Token-set overlap between the input title and this record's title, 0–1.
    /// 0.0 when no title was supplied.
    pub title_similarity: f64,
    pub attachments: Vec<CandidateAttachment>,
    pub triage: Triage,
    pub triage_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_diff: Option<MetadataDiff>,
    pub default_action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindDuplicatesResult {
    /// Every query the tool ran, with row counts. This is the audit trail the
    /// caller used to have to echo by hand.
    pub queries_run: Vec<QueryRun>,
    pub candidates: Vec<DuplicateCandidate>,
    /// Other candidates that look like thin duplicates of the aborting one.
    pub possible_stub_duplicates: Vec<String>,
    pub recommendation: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step_if_empty: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure text layer
// ---------------------------------------------------------------------------

/// Reduce a raw title or filename to comparable words.
///
/// Order matters: extension, then copy counters, then bracketed groups (which is
/// what removes shadow-library noise like `(z-lib.org)`), then punctuation to
/// spaces, then collapse, then lowercase.
pub fn normalise_title(raw: &str) -> String {
    let mut s = raw.trim().to_string();

    let lower = s.to_ascii_lowercase();
    for ext in EBOOK_EXTENSIONS {
        if lower.ends_with(ext) {
            s.truncate(s.len() - ext.len());
            break;
        }
    }

    // Bracketed groups: (z-lib.org), [nodrm], (1lib.sk), (Yakov Rabkin).
    s = strip_bracketed(&s);

    // Trailing copy counters, possibly repeated: "title-1", "title copy 2".
    let mut prev = String::new();
    while prev != s {
        prev = s.clone();
        let t = s.trim_end();
        let t = t.strip_suffix(" copy").unwrap_or(t);
        let t = match t.rsplit_once(" copy ") {
            Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head,
            _ => t,
        };
        let t = match t.rsplit_once(['-', '_']) {
            Some((head, tail))
                if !head.is_empty()
                    && !tail.is_empty()
                    && tail.chars().all(|c| c.is_ascii_digit()) =>
            {
                head
            }
            _ => t,
        };
        s = t.to_string();
    }

    let flattened: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();

    flattened
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn strip_bracketed(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth_paren = 0usize;
    let mut depth_square = 0usize;
    for c in s.chars() {
        match c {
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '[' => depth_square += 1,
            ']' => depth_square = depth_square.saturating_sub(1),
            _ if depth_paren == 0 && depth_square == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Words worth searching on: normalised, at least 4 characters, not a stop
/// word, deduped, longest first (the longest word is the most selective thing
/// we can hand a `LIKE`).
pub fn significant_tokens(raw: &str) -> Vec<String> {
    let norm = normalise_title(raw);
    let mut toks: Vec<String> = Vec::new();
    for w in norm.split_whitespace() {
        let w = w.trim_matches('\'');
        if w.chars().count() < 4 || STOP_WORDS.contains(&w) {
            continue;
        }
        let owned = w.to_string();
        if !toks.contains(&owned) {
            toks.push(owned);
        }
    }
    // Stable sort, so equal-length words keep the order they appear in the title.
    toks.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    toks
}

/// Jaccard overlap of two token sets. Zero when either side is empty — an
/// unknown title is not a match, it is an absence of evidence.
pub fn title_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = shared_token_count(a, b);
    let union = a.len() + b.len() - shared;
    if union == 0 {
        0.0
    } else {
        shared as f64 / union as f64
    }
}

pub fn shared_token_count(a: &[String], b: &[String]) -> usize {
    a.iter().filter(|t| b.contains(t)).count()
}

/// Every form of the identifier worth querying the library for.
fn identifier_variants(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    let mut out: Vec<String> = if isbn::looks_like_isbn(trimmed) {
        isbn::isbn_variants(trimmed)
    } else {
        vec![
            trimmed.to_string(),
            crate::core::identifier::normalise_doi(trimmed),
            crate::core::identifier::normalise_arxiv_id(trimmed),
        ]
    };
    out.dedup();
    let mut seen = Vec::new();
    out.retain(|v| {
        if v.is_empty() || seen.contains(v) {
            false
        } else {
            seen.push(v.clone());
            true
        }
    });
    out
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

struct Hit {
    hit: SearchHit,
    found_by: Vec<String>,
}

/// Run the whole dedup gate. Read-only; touches nothing outside the local
/// library and does not call any external catalogue.
pub async fn find_duplicates(
    pool: &ReadOnlyPool,
    library_id: i64,
    storage_dir: &Path,
    input: &DedupInput,
) -> Result<FindDuplicatesResult> {
    input.validate().map_err(Error::Config)?;

    let limit = if input.limit > 0 {
        input.limit
    } else {
        DEFAULT_LIMIT
    };
    let input_tokens = input
        .title
        .as_deref()
        .map(significant_tokens)
        .unwrap_or_default();
    let min_shared = if input_tokens.len() <= 1 {
        1
    } else {
        MIN_SHARED_TOKENS
    };

    let mut queries_run: Vec<QueryRun> = Vec::new();
    let mut found: BTreeMap<String, Hit> = BTreeMap::new();

    // Pass A — title tokens, longest (most selective) first. One query per
    // token: if the record happens not to contain our most selective word, the
    // shorter ones still give it a chance, for one cheap local query each.
    for token in input_tokens.iter().take(MAX_TITLE_QUERIES) {
        let hits = query(pool, library_id, token, limit).await?;
        let mut kept = 0;
        for h in &hits {
            let cand_tokens = h
                .title
                .as_deref()
                .map(significant_tokens)
                .unwrap_or_default();
            if shared_token_count(&input_tokens, &cand_tokens) >= min_shared {
                record(&mut found, h, "title");
                kept += 1;
            }
        }
        queries_run.push(QueryRun {
            pass: "title".into(),
            query: token.clone(),
            result_count: hits.len(),
            kept,
        });
    }

    // Pass B — author surname, then filtered by title. Without the filter this
    // returns everything the author wrote.
    if let Some(surname) = input.author_surname.as_deref().map(str::trim) {
        if !surname.is_empty() {
            let hits = query(pool, library_id, surname, limit).await?;
            let mut kept = 0;
            for h in &hits {
                if !input_tokens.is_empty() {
                    let cand_tokens = h
                        .title
                        .as_deref()
                        .map(significant_tokens)
                        .unwrap_or_default();
                    if shared_token_count(&input_tokens, &cand_tokens) == 0 {
                        continue;
                    }
                }
                record(&mut found, h, "author");
                kept += 1;
            }
            queries_run.push(QueryRun {
                pass: "author".into(),
                query: surname.to_string(),
                result_count: hits.len(),
                kept,
            });
        }
    }

    // Pass C — identifier, in every plausible form. An identifier match is
    // strong evidence on its own, so no title filter applies.
    if let Some(id) = input.identifier.as_deref() {
        for form in identifier_variants(id) {
            let hits = query(pool, library_id, &form, limit).await?;
            for h in &hits {
                record(&mut found, h, "identifier");
            }
            queries_run.push(QueryRun {
                pass: "identifier".into(),
                query: form,
                result_count: hits.len(),
                kept: hits.len(),
            });
        }
    }

    // Trash: never offer a deleted item as a duplicate.
    let all_keys: Vec<String> = found.keys().cloned().collect();
    let trashed = trashed_keys(pool, library_id, &all_keys).await?;
    found.retain(|k, _| !trashed.contains(k));

    // Hydrate.
    let mut candidates = Vec::new();
    for (key, entry) in &found {
        let item = get_item_by_key(pool, key, library_id).await?;
        let atts = list_attachments(pool, key, library_id, storage_dir).await?;
        candidates.push(build_candidate(
            entry,
            &item,
            &atts,
            &input_tokens,
            input.identifier.as_deref(),
            input.input_kind,
        ));
    }

    // Strongest evidence first, so the caller reads the important one.
    candidates.sort_by(|a, b| {
        triage_rank(a.triage)
            .cmp(&triage_rank(b.triage))
            .then(b.title_similarity.total_cmp(&a.title_similarity))
    });

    let (recommendation, possible_stub_duplicates) = resolve_recommendation(&candidates);
    let next_step_if_empty = candidates
        .is_empty()
        .then(|| "no candidates — safe to create a new item".to_string());

    Ok(FindDuplicatesResult {
        queries_run,
        candidates,
        possible_stub_duplicates,
        recommendation,
        next_step_if_empty,
    })
}

async fn query(
    pool: &ReadOnlyPool,
    library_id: i64,
    q: &str,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    search_metadata(
        pool,
        library_id,
        SearchParams {
            query: q.to_string(),
            include_fulltext: false,
            limit,
            ..Default::default()
        },
    )
    .await
}

fn record(found: &mut BTreeMap<String, Hit>, h: &SearchHit, pass: &str) {
    let e = found.entry(h.key.clone()).or_insert_with(|| Hit {
        hit: h.clone(),
        found_by: Vec::new(),
    });
    if !e.found_by.iter().any(|p| p == pass) {
        e.found_by.push(pass.to_string());
    }
}

fn triage_rank(t: Triage) -> u8 {
    match t {
        Triage::I => 0,
        Triage::Ii => 1,
        Triage::Iii => 2,
    }
}

/// Resolve possibly-conflicting per-candidate defaults into one recommendation.
/// Precedence: any `i` (it's already here, complete) beats any `ii` (it's here
/// but wants our file) beats any `iii` (not sure). Never contradictory.
pub fn resolve_recommendation(candidates: &[DuplicateCandidate]) -> (Action, Vec<String>) {
    if candidates.is_empty() {
        return (Action::CreateNew, vec![]);
    }
    if let Some(winner) = candidates.iter().find(|c| c.triage == Triage::I) {
        let stubs = candidates
            .iter()
            .filter(|c| c.item_key != winner.item_key)
            .map(|c| c.item_key.clone())
            .collect();
        return (Action::Abort, stubs);
    }
    if candidates.iter().any(|c| c.triage == Triage::Ii) {
        return (Action::AttachToExisting, vec![]);
    }
    (Action::Ask, vec![])
}

fn build_candidate(
    entry: &Hit,
    item: &Item,
    atts: &[crate::core::types::Attachment],
    input_tokens: &[String],
    input_identifier: Option<&str>,
    kind: InputKind,
) -> DuplicateCandidate {
    let cand_tokens = entry
        .hit
        .title
        .as_deref()
        .map(significant_tokens)
        .unwrap_or_default();
    let similarity = title_similarity(input_tokens, &cand_tokens);

    let attachments: Vec<CandidateAttachment> = atts
        .iter()
        .map(|a| CandidateAttachment {
            link_mode: a.link_mode,
            content_type: a.content_type.clone(),
            filename: a.filename.clone(),
        })
        .collect();

    let has_pdf = atts.iter().any(|a| {
        matches!(
            a.link_mode,
            AttachmentLinkMode::ImportedFile | AttachmentLinkMode::LinkedFile
        ) && a.content_type.as_deref() == Some("application/pdf")
    });
    let has_web = atts.iter().any(|a| {
        matches!(
            a.link_mode,
            AttachmentLinkMode::ImportedUrl | AttachmentLinkMode::LinkedUrl
        )
    });

    let identifier_conflict = input_identifier
        .map(|given| conflicting_identifier(item, given))
        .unwrap_or(false);

    // Weak agreement or a clashing identifier means the tool is not confident,
    // whatever the attachments say.
    let (triage, reason) = if identifier_conflict {
        (
            Triage::Iii,
            "candidate carries a different identifier from the one supplied".to_string(),
        )
    } else if !input_tokens.is_empty() && similarity < WEAK_SIMILARITY {
        (
            Triage::Iii,
            format!("titles only partly agree (similarity {similarity:.2})"),
        )
    } else {
        match kind {
            InputKind::Pdf if has_pdf => (
                Triage::I,
                "candidate already has a PDF attachment; input is a pdf".to_string(),
            ),
            InputKind::Pdf => (
                Triage::Ii,
                "candidate has no PDF attachment; input is a pdf".to_string(),
            ),
            InputKind::Url if has_web => (
                Triage::I,
                "candidate already has a snapshot or link; input is a url".to_string(),
            ),
            InputKind::Url => (
                Triage::Ii,
                "candidate has no snapshot or link; input is a url".to_string(),
            ),
            InputKind::Name => (
                Triage::I,
                "candidate matches and there is nothing to attach".to_string(),
            ),
        }
    };

    let default_action = match triage {
        Triage::I => Action::Abort,
        Triage::Ii => Action::AttachToExisting,
        Triage::Iii => Action::Ask,
    };

    let metadata_diff = (triage == Triage::Ii).then(|| sparseness(item));

    DuplicateCandidate {
        item_key: entry.hit.key.clone(),
        citation_key: item.citation_key.clone(),
        title: entry.hit.title.clone(),
        year: entry.hit.year.clone(),
        creators_short: creators_short(item),
        item_type: item.item_type.clone(),
        found_by: entry.found_by.clone(),
        title_similarity: similarity,
        attachments,
        triage,
        triage_reason: reason,
        metadata_diff,
        default_action,
    }
}

fn creators_short(item: &Item) -> Option<String> {
    let first = item.creators.first()?;
    let name = first
        .last_name
        .clone()
        .or_else(|| first.first_name.clone())?;
    Some(if item.creators.len() > 1 {
        format!("{name} et al.")
    } else {
        name
    })
}

/// Does the candidate carry an identifier of the same kind as the one supplied,
/// with a different value? Absence is not a conflict — most records have no DOI.
fn conflicting_identifier(item: &Item, given: &str) -> bool {
    let field = if isbn::looks_like_isbn(given) {
        "ISBN"
    } else {
        "DOI"
    };
    let Some(existing) = item.fields.get(field).and_then(|v| v.as_str()) else {
        return false;
    };
    if field == "ISBN" {
        let given_forms = isbn::isbn_variants(given);
        let existing_norm = isbn::normalise_isbn(existing);
        !given_forms.contains(&existing_norm)
    } else {
        !existing.eq_ignore_ascii_case(&crate::core::identifier::normalise_doi(given))
    }
}

/// What the existing record lacks. A heuristic prompt, not a diff: the tool
/// cannot know what the caller is holding, only what is thin here.
fn sparseness(item: &Item) -> MetadataDiff {
    let mut missing = Vec::new();
    let mut thin = Vec::new();
    for f in SPARSENESS_FIELDS {
        match item.fields.get(*f).and_then(|v| v.as_str()) {
            None => missing.push((*f).to_string()),
            Some(v) if v.trim().is_empty() => missing.push((*f).to_string()),
            Some(v) => {
                if *f == "date" && v.trim().len() == 4 {
                    thin.push("date".to_string());
                }
            }
        }
    }
    MetadataDiff { missing, thin }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_title_strips_extension_counters_and_shadow_library_noise() {
        assert_eq!(
            normalise_title("What is Modern Israel (Yakov Rabkin) (z-lib.org).pdf"),
            "what is modern israel"
        );
        assert_eq!(normalise_title("Some Book [nodrm].epub"), "some book");
        assert_eq!(normalise_title("Some Book-1.pdf"), "some book");
        assert_eq!(normalise_title("Some Book copy 2.pdf"), "some book");
        assert_eq!(normalise_title("Some_Book_Title.mobi"), "some book title");
    }

    /// The 2026-05-13 punctuation failure, at the unit level: the two forms must
    /// normalise to the same string, so token comparison cannot be defeated by
    /// a colon on either side.
    #[test]
    fn normalise_title_flattens_punctuation() {
        assert_eq!(
            normalise_title("Gaza: An inquest into its martyrdom"),
            normalise_title("Gaza An Inquest Into Its Martyrdom")
        );
        assert_eq!(
            normalise_title("Israel—Palestine: A Study; Vol. 2"),
            "israel palestine a study vol 2"
        );
    }

    #[test]
    fn significant_tokens_drops_stop_words_and_short_words() {
        assert_eq!(
            significant_tokens("What is Modern Israel?"),
            // Equal length, so the sort is stable and title order survives.
            vec!["modern", "israel"]
        );
        assert_eq!(
            significant_tokens("Gaza: An inquest into its martyrdom"),
            vec!["martyrdom", "inquest", "gaza"]
        );
    }

    #[test]
    fn significant_tokens_are_longest_first_and_deduped() {
        let t = significant_tokens("Palestine Palestine Israel Occupation");
        assert_eq!(t, vec!["occupation", "palestine", "israel"]);
    }

    #[test]
    fn title_similarity_is_jaccard_and_zero_on_empty() {
        let a = significant_tokens("Modern Israel Studies Handbook Companion");
        let b = significant_tokens("What is Modern Israel?");
        // {modern, israel} shared of {modern, israel, studies, handbook, companion}
        assert!((title_similarity(&a, &b) - 0.4).abs() < 1e-9);
        assert_eq!(title_similarity(&[], &b), 0.0);
    }

    #[test]
    fn identical_titles_score_one() {
        let a = significant_tokens("Gaza An Inquest Into Its Martyrdom");
        let b = significant_tokens("Gaza: An inquest into its martyrdom");
        assert!((title_similarity(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn identifier_variants_covers_both_isbn_forms() {
        assert_eq!(
            identifier_variants("1-844674-87-8"),
            vec!["1844674878", "9781844674879"]
        );
        assert_eq!(
            identifier_variants("https://doi.org/10.1234/abcd"),
            vec!["https://doi.org/10.1234/abcd", "10.1234/abcd"]
        );
    }

    #[test]
    fn validate_requires_one_usable_input() {
        assert!(DedupInput::default().validate().is_err());
        assert!(DedupInput {
            title: Some("   ".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(DedupInput {
            author_surname: Some("Rabkin".into()),
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    fn candidate(key: &str, triage: Triage) -> DuplicateCandidate {
        DuplicateCandidate {
            item_key: key.into(),
            citation_key: None,
            title: None,
            year: None,
            creators_short: None,
            item_type: "book".into(),
            found_by: vec!["title".into()],
            title_similarity: 1.0,
            attachments: vec![],
            triage,
            triage_reason: String::new(),
            metadata_diff: None,
            default_action: match triage {
                Triage::I => Action::Abort,
                Triage::Ii => Action::AttachToExisting,
                Triage::Iii => Action::Ask,
            },
        }
    }

    #[test]
    fn recommendation_precedence_i_beats_ii_beats_iii() {
        let (a, stubs) = resolve_recommendation(&[
            candidate("B", Triage::Ii),
            candidate("A", Triage::I),
            candidate("C", Triage::Iii),
        ]);
        assert_eq!(a, Action::Abort);
        assert_eq!(stubs, vec!["B".to_string(), "C".to_string()]);

        let (a, stubs) =
            resolve_recommendation(&[candidate("C", Triage::Iii), candidate("B", Triage::Ii)]);
        assert_eq!(a, Action::AttachToExisting);
        assert!(stubs.is_empty());

        assert_eq!(
            resolve_recommendation(&[candidate("C", Triage::Iii)]).0,
            Action::Ask
        );
        assert_eq!(resolve_recommendation(&[]).0, Action::CreateNew);
    }
}
