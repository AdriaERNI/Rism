//! Shared name-validation predicates. Moved verbatim from
//! `src/tools/testing.rs` (`E_SPEC` §3) so every interpolation site —
//! tests, graph SQL — shares ONE definition of the security predicate.

/// Class-name allowlist (package.Class form, optional leading %).
/// The query endpoint has no bind parameters, so interpolation must be
/// validated — same rule Prism enforces.
pub(crate) fn valid_class_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '%')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// Plain identifier allowlist (routine/label form: letter first, then
/// alphanumerics/underscore).
pub(crate) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Namespace allowlist (`%SYS`/`USER`/`MY-APP` shapes): the namespace rides
/// UNENCODED into the URL path (`src/iris/http.rs::api_url` only pre-encodes
/// `%`), so it gets the same client-side guard as class names — refuse before
/// any request instead of trusting path normalization (`F_FINDINGS` F-09).
pub(crate) fn valid_namespace(ns: &str) -> bool {
    let rest = ns.strip_prefix('%').unwrap_or(ns);
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return false; // "" or bare "%" — no real namespace has an empty tail
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    /// U15 — the moved predicate keeps the old acceptance set
    /// (same cases as the historical `testing.rs` test).
    #[test]
    fn names_valid_class_name_moved() {
        assert!(valid_class_name("My.Tests.Calc"));
        assert!(valid_class_name("%UnitTest.SQLRegression"));
        assert!(!valid_class_name("Robert'; DROP TABLE"));
        assert!(!valid_class_name("Class.Name--x"));
        assert!(!valid_class_name(""));
        assert!(valid_identifier("TestAdd"));
        assert!(!valid_identifier("Test Add"));
        assert!(!valid_identifier("1Test"));
    }

    /// F-09 pin — the namespace predicate accepts every real IRIS name
    /// shape and refuses every path/param metachar seen in H-10..H-13.
    #[test]
    fn namespace_allowlist_shape() {
        assert!(valid_namespace("USER"));
        assert!(valid_namespace("%SYS"));
        assert!(valid_namespace("MY-APP"));
        assert!(valid_namespace("Ensemble_1"));
        assert!(!valid_namespace(""));
        assert!(!valid_namespace("A' OR 1=1--"));
        assert!(!valid_namespace("USER&x=1"));
        assert!(!valid_namespace("../../"));
        assert!(!valid_namespace("../admin"));
        assert!(!valid_namespace("USER/%2e%2e"));
        // `%` must be followed by a letter (%SYS shape): a bare `%`,
        // `%%`, or `%41` (pre-encoded garbage) is a client-side refusal,
        // not a server round trip.
        assert!(!valid_namespace("%41"));
        assert!(!valid_namespace("%%"));
        assert!(!valid_namespace("%"));
        assert!(!valid_namespace("USER/"));
    }
}
