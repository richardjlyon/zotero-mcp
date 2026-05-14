#[derive(Debug, Clone, Default)]
pub struct PdfSignals {
    pub doi_candidates: Vec<String>,
    pub arxiv_candidates: Vec<String>,
    pub isbn_candidates: Vec<String>,
    pub title_candidate: Option<String>,
    pub author_candidates: Vec<String>,
}

pub fn extract_signals(text: &str) -> PdfSignals {
    // First-page bias: cap to first 4000 chars.
    let head: String = text.chars().take(4000).collect();

    PdfSignals {
        doi_candidates: find_dois(&head),
        arxiv_candidates: find_arxiv(&head),
        isbn_candidates: find_isbn(&head),
        title_candidate: guess_title(&head),
        author_candidates: guess_authors(&head),
    }
}

fn find_dois(s: &str) -> Vec<String> {
    let mut out = vec![];
    for token in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == ')') {
        let t = token
            .trim_start_matches("doi:")
            .trim_start_matches("DOI:")
            .trim();
        if t.starts_with("10.") && t.contains('/') && t.len() < 200 {
            // Strip trailing punctuation.
            let cleaned = t.trim_end_matches(|c: char| !c.is_alphanumeric());
            if !out.iter().any(|x: &String| x == cleaned) {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn find_arxiv(s: &str) -> Vec<String> {
    let mut out = vec![];
    for needle in ["arXiv:", "arxiv:"] {
        let mut rest = s;
        while let Some(i) = rest.find(needle) {
            let after = &rest[i + needle.len()..];
            let id: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !id.is_empty() && id.contains('.') && !out.contains(&id) {
                out.push(id.clone());
            }
            rest = &after[id.len()..];
        }
    }
    out
}

fn find_isbn(s: &str) -> Vec<String> {
    let mut out = vec![];
    for w in s.split_whitespace() {
        let digits: String = w
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X')
            .collect();
        if (digits.len() == 10 || digits.len() == 13) && !out.contains(&digits) {
            out.push(digits);
        }
    }
    out
}

fn guess_title(s: &str) -> Option<String> {
    for line in s.lines() {
        let t = line.trim();
        if t.len() > 12
            && t.split_whitespace().count() >= 3
            && !t.starts_with("DOI")
            && !t.starts_with("doi:")
        {
            return Some(t.to_string());
        }
    }
    None
}

fn guess_authors(s: &str) -> Vec<String> {
    // After the title line, the next non-empty line is a heuristic author list.
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return vec![];
    }
    let line = lines[1];
    line.split(|c: char| c == ',' || c == ';')
        .map(|s| s.trim().to_string())
        .filter(|s| s.split_whitespace().count() >= 2 && s.len() < 60)
        .collect()
}
