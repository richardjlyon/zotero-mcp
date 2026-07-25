//! Wire-shape types for the tool surface.
//!
//! These exist for the MCP boundary, not for the domain — nothing in `core/`
//! produces or consumes them, which is why they live here rather than in
//! `core/types.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A list-returning tool's response.
///
/// Every list tool returns this same shape, so a caller that has learned one has
/// learned all nine. The envelope is required rather than decorative: the MCP
/// spec demands an object at the root of a tool's `outputSchema`, and rmcp
/// enforces it by panicking at startup — a bare `Vec<T>` schema is `type:
/// "array"` and cannot be advertised at all.
///
/// Having been forced into an envelope, we get something worth having out of it:
/// `count` and `possibly_truncated` answer a question a bare array could not.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    /// How many items this response carries.
    pub count: usize,
    /// True when the response filled the limit that was applied, so the library
    /// may hold more matches than are shown. Ask again with a higher `limit` or
    /// an `offset`.
    ///
    /// Deliberately *possibly*: a result that exactly fills the limit is
    /// indistinguishable from one that was cut off, and resolving that for
    /// certain would mean a second counting query on every call.
    pub possibly_truncated: bool,
}

impl<T> ListResult<T> {
    /// For tools that return everything they find, with no limit to hit.
    pub fn complete(items: Vec<T>) -> Self {
        Self {
            count: items.len(),
            items,
            possibly_truncated: false,
        }
    }

    /// For tools that applied a row limit. Pass the *effective* limit — the one
    /// actually used, after any default was substituted.
    pub fn with_limit(items: Vec<T>, effective_limit: i64) -> Self {
        let count = items.len();
        let possibly_truncated = effective_limit > 0 && count as i64 >= effective_limit;
        Self {
            items,
            count,
            possibly_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_never_flags_truncation() {
        let r = ListResult::complete(vec![1, 2, 3]);
        assert_eq!(r.count, 3);
        assert!(!r.possibly_truncated);
    }

    #[test]
    fn with_limit_flags_at_the_boundary() {
        // Short of the limit: definitely everything.
        assert!(!ListResult::with_limit(vec![1, 2], 5).possibly_truncated);
        // Exactly at the limit: might be more, cannot know.
        assert!(ListResult::with_limit(vec![1, 2], 2).possibly_truncated);
    }

    #[test]
    fn with_limit_treats_a_non_positive_limit_as_no_limit() {
        // Guards against a caller passing the raw arg before the default is
        // applied: "limit 0" must not mean "everything is truncated".
        assert!(!ListResult::with_limit(vec![1, 2, 3], 0).possibly_truncated);
        assert!(!ListResult::with_limit(vec![1, 2, 3], -1).possibly_truncated);
    }

    #[test]
    fn empty_is_not_truncated() {
        let r: ListResult<u8> = ListResult::with_limit(vec![], 10);
        assert_eq!(r.count, 0);
        assert!(!r.possibly_truncated);
    }

    #[test]
    fn count_always_matches_items() {
        let r = ListResult::with_limit(vec!["a", "b", "c"], 50);
        assert_eq!(r.count, r.items.len());
    }
}
