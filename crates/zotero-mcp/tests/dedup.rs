//! Integration tests for the `find_duplicates` composite dedup gate.
//!
//! Every test here is named for a real failure from the 2026-05-13
//! `adding-references` eval pass, or for a rule those failures forced.

mod fixtures;

use zotero_mcp::core::dedup::{find_duplicates, Action, DedupInput, InputKind, Triage};
use zotero_mcp::core::reader::pool::ReadOnlyPool;

struct Harness {
    _f: fixtures::build_fixture::Fixture,
    pool: ReadOnlyPool,
    storage_dir: std::path::PathBuf,
}

async fn harness() -> Harness {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let storage_dir = f.storage_dir();
    Harness {
        _f: f,
        pool,
        storage_dir,
    }
}

impl Harness {
    async fn run(&self, input: DedupInput) -> zotero_mcp::core::dedup::FindDuplicatesResult {
        find_duplicates(&self.pool, 1, &self.storage_dir, &input)
            .await
            .expect("find_duplicates should not error")
    }
}

fn keys(r: &zotero_mcp::core::dedup::FindDuplicatesResult) -> Vec<String> {
    r.candidates.iter().map(|c| c.item_key.clone()).collect()
}

// ---------------------------------------------------------------------------
// The punctuation case (2026-05-13)
// ---------------------------------------------------------------------------

/// `search_items("Gaza An Inquest Into Its Martyrdom")` returns nothing against
/// a record stored as `Gaza: An inquest into its martyrdom`, because the search
/// is a single SQL `LIKE '%<whole query>%'` and the colon is on the *database*
/// side. Normalising only the query cannot fix that. Token matching can.
#[tokio::test]
async fn gaza_case_token_search_finds_the_colon_titled_record() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("Gaza An Inquest Into Its Martyrdom".into()),
            author_surname: None,
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    assert!(
        keys(&r).contains(&"GAZA0001".to_string()),
        "expected GAZA0001 among candidates, got {:?} (queries: {:?})",
        keys(&r),
        r.queries_run
    );
}

/// Control for the test above: the naive single-substring query really does
/// miss, so the token pass is load-bearing rather than belt-and-braces.
#[tokio::test]
async fn gaza_case_single_substring_search_still_misses() {
    let h = harness().await;
    let hits = zotero_mcp::core::reader::search::search_metadata(
        &h.pool,
        1,
        zotero_mcp::core::reader::search::SearchParams {
            query: "Gaza An Inquest Into Its Martyrdom".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        !hits.iter().any(|x| x.key == "GAZA0001"),
        "if this now passes, search_items has changed and the token pass may be revisited"
    );
}

// ---------------------------------------------------------------------------
// The "Yakob" case (2026-05-13)
// ---------------------------------------------------------------------------

/// The duplicate that slipped through: the stored record has the author's first
/// name misspelt ("Yakob" for "Yakov"). A surname pass is unaffected by that,
/// which is why the author pass is mandatory rather than a fallback.
#[tokio::test]
async fn yakob_case_author_pass_finds_record_with_misspelt_first_name() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("What is Modern Israel".into()),
            author_surname: Some("Rabkin".into()),
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    let c = r
        .candidates
        .iter()
        .find(|c| c.item_key == "JGF2UTMW")
        .unwrap_or_else(|| panic!("expected JGF2UTMW, got {:?}", keys(&r)));
    assert!(
        c.found_by.iter().any(|p| p == "author"),
        "expected the author pass to have surfaced it, found_by = {:?}",
        c.found_by
    );
}

/// A surname search returns everything the author wrote. Without a title filter
/// the gate floods: the observed case is an input of "What is Modern Israel"
/// surfacing "Gaza: An Inquest into its Martyrdom" — same author, no shared
/// content word.
#[tokio::test]
async fn author_pass_discards_unrelated_book_by_same_author() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("What is Modern Israel".into()),
            author_surname: Some("Rabkin".into()),
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    assert!(
        !keys(&r).contains(&"GAZA0001".to_string()),
        "Gaza record must not be a candidate for a Modern Israel input; got {:?}",
        keys(&r)
    );
}

// ---------------------------------------------------------------------------
// Trash
// ---------------------------------------------------------------------------

/// A trashed item resurfacing as a duplicate would cause a spurious abort.
/// Nothing in the reader layer filtered `deletedItems` before this change.
#[tokio::test]
async fn trashed_items_are_never_candidates() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("What is Modern Israel".into()),
            author_surname: Some("Rabkin".into()),
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    assert!(
        !keys(&r).contains(&"TRSH0001".to_string()),
        "trashed item leaked into candidates: {:?}",
        keys(&r)
    );
}

// ---------------------------------------------------------------------------
// Triage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn triage_i_when_candidate_already_has_pdf_and_input_is_pdf() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("A Paper on Things".into()),
            author_surname: None,
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    let c = r
        .candidates
        .iter()
        .find(|c| c.item_key == "AAAA0001")
        .unwrap_or_else(|| panic!("expected AAAA0001, got {:?}", keys(&r)));
    assert_eq!(c.triage, Triage::I, "reason: {}", c.triage_reason);
    assert_eq!(c.default_action, Action::Abort);
    assert_eq!(r.recommendation, Action::Abort);
}

#[tokio::test]
async fn triage_ii_when_candidate_has_no_pdf() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("What is Modern Israel".into()),
            author_surname: Some("Rabkin".into()),
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    let c = r
        .candidates
        .iter()
        .find(|c| c.item_key == "JGF2UTMW")
        .unwrap();
    assert_eq!(c.triage, Triage::Ii, "reason: {}", c.triage_reason);
    assert_eq!(c.default_action, Action::AttachToExisting);
    assert_eq!(r.recommendation, Action::AttachToExisting);
    // The sparseness heuristic: the fixture record has no place and no abstract.
    let diff = c
        .metadata_diff
        .as_ref()
        .expect("triage ii carries a metadata_diff");
    assert!(diff.missing.iter().any(|f| f == "place"), "{diff:?}");
    assert!(
        !diff.missing.iter().any(|f| f == "ISBN"),
        "the fixture record HAS an ISBN: {diff:?}"
    );
}

#[tokio::test]
async fn triage_iii_on_weak_similarity() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("Modern Israel Studies Handbook Companion".into()),
            author_surname: None,
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    let c = r
        .candidates
        .iter()
        .find(|c| c.item_key == "JGF2UTMW")
        .unwrap_or_else(|| panic!("expected JGF2UTMW, got {:?}", keys(&r)));
    assert_eq!(c.triage, Triage::Iii, "similarity {}", c.title_similarity);
    assert_eq!(c.default_action, Action::Ask);
    assert_eq!(r.recommendation, Action::Ask);
}

// ---------------------------------------------------------------------------
// Identifier pass
// ---------------------------------------------------------------------------

/// The fixture stores the ISBN-13. A caller holding the ISBN-10 printed on an
/// older paperback must still find the record.
#[tokio::test]
async fn identifier_pass_tries_both_isbn_forms() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: None,
            author_surname: None,
            identifier: Some("1-844674-87-8".into()),
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    assert!(
        keys(&r).contains(&"JGF2UTMW".to_string()),
        "ISBN-10 input should find the ISBN-13 record; queries: {:?}",
        r.queries_run
    );
}

// ---------------------------------------------------------------------------
// Empty result
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_candidates_recommends_create_new() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("Quixotic Treatise Concerning Nonexistent Windmills".into()),
            author_surname: None,
            identifier: None,
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    assert!(r.candidates.is_empty(), "got {:?}", keys(&r));
    assert_eq!(r.recommendation, Action::CreateNew);
    assert!(r.next_step_if_empty.is_some());
}

/// Every query the tool ran is reported, with its row count — the tool's output
/// *is* the echo the skill previously had to produce by hand.
#[tokio::test]
async fn queries_run_records_every_pass() {
    let h = harness().await;
    let r = h
        .run(DedupInput {
            title: Some("What is Modern Israel".into()),
            author_surname: Some("Rabkin".into()),
            identifier: Some("9781844674879".into()),
            input_kind: InputKind::Pdf,
            limit: 0,
        })
        .await;
    for pass in ["title", "author", "identifier"] {
        assert!(
            r.queries_run.iter().any(|q| q.pass == pass),
            "no {pass} query recorded in {:?}",
            r.queries_run
        );
    }
}
