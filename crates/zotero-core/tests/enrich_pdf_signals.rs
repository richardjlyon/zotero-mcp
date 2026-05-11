use zotero_core::enrichment::pdf_signals::extract_signals;

#[test]
fn finds_doi_in_text() {
    let text = "Some title\nDOI: 10.1234/abcd.5678  Some other text.";
    let s = extract_signals(text);
    assert_eq!(s.doi_candidates, vec!["10.1234/abcd.5678".to_string()]);
}

#[test]
fn finds_arxiv_id() {
    let text = "Preprint arXiv:2401.00001v2 available";
    let s = extract_signals(text);
    assert_eq!(s.arxiv_candidates, vec!["2401.00001".to_string()]);
}

#[test]
fn picks_first_nontrivial_line_as_title_candidate() {
    let text = "\n\nPage 1\n\nA Real Title Here\n\nAlice Aardvark, Bob Baboon\n\nAbstract: ...";
    let s = extract_signals(text);
    assert_eq!(s.title_candidate.as_deref(), Some("A Real Title Here"));
}
