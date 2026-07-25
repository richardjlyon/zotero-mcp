//! Identifier normalisation — what a caller pastes versus what a catalogue
//! wants in a URL path.
//!
//! A DOI copied from a browser arrives as `https://doi.org/10.1234/abcd`, and an
//! arXiv id as `arXiv:2401.12345`. Sent verbatim, both produce a "not found"
//! that looks like the record doesn't exist.

/// Strip URL and `doi:` wrappers from a DOI and lowercase the registrant.
///
/// The registrant prefix (`10.xxxx`) is case-insensitive per the DOI handbook;
/// the suffix is not, so it is left exactly as given even though most
/// registrars fold it in practice.
pub fn normalise_doi(raw: &str) -> String {
    let mut s = raw.trim();
    for p in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "https://www.doi.org/",
    ] {
        if s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p) {
            s = &s[p.len()..];
            break;
        }
    }
    if s.len() >= 4 && s[..4].eq_ignore_ascii_case("doi:") {
        s = s[4..].trim_start();
    }
    match s.split_once('/') {
        Some((registrant, suffix)) => format!("{}/{}", registrant.to_ascii_lowercase(), suffix),
        None => s.to_string(),
    }
}

/// Strip `arXiv:` and abs-URL wrappers. Both the new style (`2401.12345`, with
/// an optional `v2`) and the old style (`hep-th/9901001`) pass through intact.
pub fn normalise_arxiv_id(raw: &str) -> String {
    let mut s = raw.trim();
    for p in [
        "https://arxiv.org/abs/",
        "http://arxiv.org/abs/",
        "https://arxiv.org/pdf/",
    ] {
        if s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p) {
            s = &s[p.len()..];
            break;
        }
    }
    if s.len() >= 6 && s[..6].eq_ignore_ascii_case("arxiv:") {
        s = s[6..].trim_start();
    }
    s.trim_end_matches(".pdf").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_doi_strips_url_and_prefix() {
        assert_eq!(
            normalise_doi("https://doi.org/10.1234/abcd"),
            "10.1234/abcd"
        );
        assert_eq!(
            normalise_doi("http://dx.doi.org/10.1234/abcd"),
            "10.1234/abcd"
        );
        assert_eq!(normalise_doi("doi:10.1234/abcd"), "10.1234/abcd");
        assert_eq!(normalise_doi("  10.1234/abcd  "), "10.1234/abcd");
    }

    #[test]
    fn normalise_doi_lowercases_registrant_only() {
        // Registrant folded, suffix preserved: DOI suffixes are case-sensitive.
        assert_eq!(normalise_doi("10.1234/AbCd"), "10.1234/AbCd");
        assert_eq!(
            normalise_doi("HTTPS://DOI.ORG/10.1234/AbCd"),
            "10.1234/AbCd"
        );
    }

    #[test]
    fn normalise_doi_without_slash_passes_through() {
        assert_eq!(normalise_doi("not-a-doi"), "not-a-doi");
    }

    #[test]
    fn normalise_arxiv_strips_prefix_and_keeps_old_style() {
        assert_eq!(normalise_arxiv_id("arXiv:2401.12345"), "2401.12345");
        assert_eq!(normalise_arxiv_id("ARXIV: 2401.12345"), "2401.12345");
        assert_eq!(normalise_arxiv_id("2401.12345v2"), "2401.12345v2");
        assert_eq!(normalise_arxiv_id("hep-th/9901001"), "hep-th/9901001");
        assert_eq!(
            normalise_arxiv_id("https://arxiv.org/abs/2401.12345"),
            "2401.12345"
        );
    }
}
