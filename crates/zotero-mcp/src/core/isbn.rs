//! ISBN normalisation and form conversion.
//!
//! Two callers, one reason. `lookup_isbn` needs the alternate form because
//! OpenLibrary indexes some editions under the ISBN-10 and some under the
//! ISBN-13, while the caller has whichever one is printed on the book.
//! `find_duplicates` needs it because a library record may carry either form.

/// Strip hyphens, spaces and other separators; uppercase a trailing `x` check
/// digit. Does not validate.
pub fn normalise_isbn(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// ISBN-10 → ISBN-13 by prepending the `978` Bookland prefix and recomputing
/// the check digit. Returns `None` unless the input is a 10-character ISBN.
pub fn isbn10_to_13(isbn: &str) -> Option<String> {
    let n = normalise_isbn(isbn);
    if n.len() != 10 || !n[..9].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let body = format!("978{}", &n[..9]);
    Some(format!("{body}{}", check_digit_13(&body)?))
}

/// ISBN-13 → ISBN-10 by stripping a `978` prefix and recomputing the check
/// digit. Returns `None` for any other prefix: a `979`-prefixed ISBN-13 has no
/// ISBN-10 equivalent at all, and inventing one would send a lookup somewhere
/// wrong.
pub fn isbn13_to_10(isbn: &str) -> Option<String> {
    let n = normalise_isbn(isbn);
    if n.len() != 13 || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let body = n.strip_prefix("978")?;
    let body = &body[..9];
    Some(format!("{body}{}", check_digit_10(body)?))
}

/// Every plausible form of the given ISBN, most-likely first: the normalised
/// input, then its converted counterpart. Deduped; never empty for a non-empty
/// input.
pub fn isbn_variants(raw: &str) -> Vec<String> {
    let n = normalise_isbn(raw);
    let mut out = Vec::new();
    if n.is_empty() {
        return out;
    }
    out.push(n.clone());
    let alt = match n.len() {
        10 => isbn10_to_13(&n),
        13 => isbn13_to_10(&n),
        _ => None,
    };
    if let Some(a) = alt {
        if a != n {
            out.push(a);
        }
    }
    out
}

/// Does this look like an ISBN at all (10 or 13 characters after normalising)?
pub fn looks_like_isbn(raw: &str) -> bool {
    let n = normalise_isbn(raw);
    matches!(n.len(), 10 | 13) && n[..n.len() - 1].chars().all(|c| c.is_ascii_digit())
}

/// Mod-10 check digit over the first 12 digits of an ISBN-13 (weights 1,3,…).
fn check_digit_13(body12: &str) -> Option<char> {
    if body12.len() != 12 {
        return None;
    }
    let mut sum = 0u32;
    for (i, c) in body12.chars().enumerate() {
        let d = c.to_digit(10)?;
        sum += if i % 2 == 0 { d } else { d * 3 };
    }
    let check = (10 - (sum % 10)) % 10;
    char::from_digit(check, 10)
}

/// Mod-11 check digit over the first 9 digits of an ISBN-10 (weights 10…2).
/// A remainder of 10 is written `X`.
fn check_digit_10(body9: &str) -> Option<char> {
    if body9.len() != 9 {
        return None;
    }
    let mut sum = 0u32;
    for (i, c) in body9.chars().enumerate() {
        let d = c.to_digit(10)?;
        sum += d * (10 - i as u32);
    }
    let check = (11 - (sum % 11)) % 11;
    if check == 10 {
        Some('X')
    } else {
        char::from_digit(check, 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The literal ISBN from the 2026-05-13 OpenLibrary failure.
    const ISBN13: &str = "9781844674879";
    const ISBN10: &str = "1844674878";

    #[test]
    fn normalise_isbn_strips_hyphens_and_uppercases_x() {
        assert_eq!(normalise_isbn("1-844674-87-8"), ISBN10);
        assert_eq!(normalise_isbn("978 1 84467 487 9"), ISBN13);
        assert_eq!(normalise_isbn("080442957x"), "080442957X");
    }

    #[test]
    fn isbn10_to_13_computes_check_digit() {
        assert_eq!(isbn10_to_13(ISBN10).as_deref(), Some(ISBN13));
        assert_eq!(isbn10_to_13("1-844674-87-8").as_deref(), Some(ISBN13));
    }

    #[test]
    fn isbn13_to_10_computes_check_digit() {
        assert_eq!(isbn13_to_10(ISBN13).as_deref(), Some(ISBN10));
    }

    #[test]
    fn isbn_conversion_round_trips() {
        let back = isbn13_to_10(&isbn10_to_13(ISBN10).unwrap()).unwrap();
        assert_eq!(back, ISBN10);
    }

    #[test]
    fn isbn10_check_digit_x_survives_conversion() {
        // 080442957X is a real ISBN-10 with an X check digit.
        let thirteen = isbn10_to_13("080442957X").unwrap();
        assert_eq!(isbn13_to_10(&thirteen).as_deref(), Some("080442957X"));
    }

    #[test]
    fn isbn13_with_979_prefix_has_no_isbn10() {
        assert!(isbn13_to_10("9791234567896").is_none());
    }

    #[test]
    fn isbn_variants_returns_both_forms_deduped() {
        assert_eq!(isbn_variants("1-844674-87-8"), vec![ISBN10, ISBN13]);
        assert_eq!(isbn_variants(ISBN13), vec![ISBN13, ISBN10]);
        // Not an ISBN length: pass the normalised form through alone.
        assert_eq!(isbn_variants("12345"), vec!["12345"]);
        assert!(isbn_variants("").is_empty());
    }

    #[test]
    fn looks_like_isbn_discriminates() {
        assert!(looks_like_isbn("978-1-84467-487-9"));
        assert!(looks_like_isbn("080442957X"));
        assert!(!looks_like_isbn("10.1234/abcd"));
        assert!(!looks_like_isbn("2401.12345"));
    }
}
