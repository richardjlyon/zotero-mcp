//! Retry and structured-failure plumbing shared by the three `lookup_*` paths.
//!
//! OpenLibrary, CrossRef and arXiv all have transient failures — 503s,
//! connection resets, DNS blips. Before this module a single blip ended the
//! lookup and handed the caller a raw HTTP string to parse in prose. The
//! deterministic part (retry once, try the other ISBN form, record what
//! happened) belongs here; the judgement (hand-build the record, or stop and
//! ask) stays with the caller, which is why the failure is machine-readable.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Backoff before the single retry of a transient failure. Short on purpose:
/// a human is waiting on the other end of the tool call.
pub const RETRY_BACKOFF: Duration = Duration::from_millis(200);

/// Fallback backoff for a 429 with no usable `Retry-After`.
pub const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(1);

/// Never wait longer than this on a `Retry-After`, however long it asks for —
/// the caller wants an answer, and "come back in an hour" is a failure.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(5);

/// One HTTP attempt and how it went.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LookupAttempt {
    /// The identifier form this attempt used — the point of recording it is to
    /// show that the alternate ISBN form was in fact tried.
    pub identifier: String,
    /// Machine-readable outcome: `http_503`, `connection_error`, `timeout`,
    /// `decode_error`, …
    pub status: String,
    pub detail: String,
    /// Whether this outcome was worth retrying.
    pub transient: bool,
}

/// Returned when every attempt failed. Deliberately a struct rather than a
/// string: the caller branches on `suggestion` instead of pattern-matching prose.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LookupFailure {
    /// Always `lookup_failed`. Present so the payload is self-describing when
    /// it reaches a model as JSON.
    pub error: String,
    /// Which catalogue: `openlibrary`, `crossref`, `arxiv`.
    pub source: String,
    /// The identifier as normalised, before any alternate-form conversion.
    pub identifier: String,
    pub attempts: Vec<LookupAttempt>,
    /// `fall_back_to_hand_built` when the catalogue simply doesn't have it or
    /// wasn't reachable; `stop_and_ask` when something is wrong with the
    /// request or our access, where hand-building would paper over a real
    /// problem.
    pub suggestion: String,
}

impl LookupFailure {
    pub fn new(source: &str, identifier: &str, attempts: Vec<LookupAttempt>) -> Self {
        let suggestion = suggestion_for(&attempts).to_string();
        Self {
            error: "lookup_failed".into(),
            source: source.to_string(),
            identifier: identifier.to_string(),
            attempts,
            suggestion,
        }
    }
}

/// Is this HTTP status worth trying again?
///
/// 5xx and 429 are the server having a moment. Every other 4xx is an answer:
/// a 404 from CrossRef means CrossRef does not have that DOI, and retrying
/// only makes the caller wait.
pub fn status_is_transient(status: u16) -> bool {
    status >= 500 || status == 429
}

/// Does anything in this attempt trail indicate a problem with our request or
/// our access, rather than a missing or unreachable record?
pub fn suggestion_for(attempts: &[LookupAttempt]) -> &'static str {
    let blocked = attempts.iter().any(|a| {
        matches!(
            a.status.as_str(),
            "http_400" | "http_401" | "http_403" | "http_422"
        )
    });
    if blocked {
        "stop_and_ask"
    } else {
        "fall_back_to_hand_built"
    }
}

/// How long to wait before the retry, honouring `Retry-After` when the server
/// sends a sane one.
pub fn retry_delay(status: u16, headers: &reqwest::header::HeaderMap) -> Duration {
    if status != 429 {
        return RETRY_BACKOFF;
    }
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|d| d.min(MAX_RETRY_AFTER))
        .unwrap_or(RATE_LIMIT_BACKOFF)
}

fn attempt_from_error(identifier: &str, e: &reqwest::Error) -> LookupAttempt {
    let (status, transient) = if e.is_timeout() {
        ("timeout", true)
    } else if e.is_connect() || e.is_request() {
        ("connection_error", true)
    } else if e.is_decode() {
        ("decode_error", false)
    } else {
        ("request_error", true)
    };
    LookupAttempt {
        identifier: identifier.to_string(),
        status: status.into(),
        detail: e.to_string(),
        transient,
    }
}

/// Record a response-body decode failure. Not transient: the server answered,
/// we simply could not read it, and asking again gets the same bytes.
pub fn decode_attempt(identifier: &str, e: &reqwest::Error) -> LookupAttempt {
    LookupAttempt {
        identifier: identifier.to_string(),
        status: "decode_error".into(),
        detail: e.to_string(),
        transient: false,
    }
}

/// GET `url`, retrying once if the failure looks transient. Every failed try
/// appends to `attempts`; a success appends nothing (there is nothing to
/// explain). Returns `None` when both tries failed.
pub async fn get_with_retry(
    http: &reqwest::Client,
    url: &str,
    identifier: &str,
    attempts: &mut Vec<LookupAttempt>,
) -> Option<reqwest::Response> {
    // Two passes at most: the original and one retry.
    for try_index in 0..2 {
        match http.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    tracing::debug!(url, identifier, %status, "lookup attempt succeeded");
                    return Some(resp);
                }
                let code = status.as_u16();
                let transient = status_is_transient(code);
                attempts.push(LookupAttempt {
                    identifier: identifier.to_string(),
                    status: format!("http_{code}"),
                    detail: format!("HTTP {status}"),
                    transient,
                });
                tracing::debug!(url, identifier, %status, transient, "lookup attempt failed");
                if transient && try_index == 0 {
                    tokio::time::sleep(retry_delay(code, resp.headers())).await;
                    continue;
                }
                return None;
            }
            Err(e) => {
                let attempt = attempt_from_error(identifier, &e);
                let transient = attempt.transient;
                tracing::debug!(url, identifier, status = %attempt.status, "lookup attempt errored");
                attempts.push(attempt);
                if transient && try_index == 0 {
                    tokio::time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
                return None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    fn attempt(status: &str) -> LookupAttempt {
        LookupAttempt {
            identifier: "x".into(),
            status: status.into(),
            detail: String::new(),
            transient: status_is_transient(status.trim_start_matches("http_").parse().unwrap_or(0)),
        }
    }

    #[test]
    fn transience_classifier_retries_server_faults_only() {
        for s in [500, 502, 503, 504, 429] {
            assert!(status_is_transient(s), "{s} should be transient");
        }
        for s in [400, 401, 403, 404, 410, 422] {
            assert!(!status_is_transient(s), "{s} should be permanent");
        }
    }

    #[test]
    fn suggestion_is_hand_built_on_not_found_or_outage() {
        assert_eq!(
            suggestion_for(&[attempt("http_404")]),
            "fall_back_to_hand_built"
        );
        assert_eq!(
            suggestion_for(&[attempt("http_503"), attempt("http_503")]),
            "fall_back_to_hand_built"
        );
    }

    #[test]
    fn suggestion_is_stop_and_ask_on_access_problems() {
        for s in ["http_400", "http_401", "http_403", "http_422"] {
            assert_eq!(suggestion_for(&[attempt(s)]), "stop_and_ask", "{s}");
        }
        // One access problem anywhere in the trail is enough.
        assert_eq!(
            suggestion_for(&[attempt("http_503"), attempt("http_403")]),
            "stop_and_ask"
        );
    }

    #[test]
    fn retry_delay_honours_retry_after_within_a_ceiling() {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(retry_delay(429, &h), Duration::from_secs(2));

        // Absurd values are clamped: the caller wants an answer.
        h.insert(RETRY_AFTER, HeaderValue::from_static("3600"));
        assert_eq!(retry_delay(429, &h), MAX_RETRY_AFTER);

        // Unparseable, or not a rate limit at all.
        h.insert(RETRY_AFTER, HeaderValue::from_static("Wed, 21 Oct 2026"));
        assert_eq!(retry_delay(429, &h), RATE_LIMIT_BACKOFF);
        assert_eq!(retry_delay(503, &HeaderMap::new()), RETRY_BACKOFF);
    }

    #[test]
    fn failure_carries_its_own_suggestion() {
        let f = LookupFailure::new("openlibrary", "9781844674879", vec![attempt("http_503")]);
        assert_eq!(f.error, "lookup_failed");
        assert_eq!(f.suggestion, "fall_back_to_hand_built");
        assert_eq!(f.identifier, "9781844674879");
    }
}
