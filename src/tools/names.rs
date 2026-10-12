//! Shared name-validation predicates. Moved verbatim from
//! `src/tools/testing.rs` (E_SPEC §3) so every interpolation site —
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
}
