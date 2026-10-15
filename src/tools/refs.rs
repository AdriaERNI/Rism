//! `find_references` — call-graph references by parse-on-fetch. IRIS 2025.3
//! ships no server cross-reference: the Atelier xref/Graph endpoints 404,
//! the legacy Studio xref globals (`^XDEF`, `^%CC`) are empty,
//! `%Library.SyntaxColor` is a tokenizer, and `%Dictionary.CompiledCode`
//! does not exist (all probed live against rism-iris; evidence trail in
//! `testing-notes/G_RESEARCH.md` §1–9). The shipped mechanism is therefore:
//!
//! 1. narrow candidate docs with `action/search` (literal mode) plus
//!    `%Dictionary` SQL over the embedded-code columns,
//! 2. fetch each candidate doc ONCE through the existing `doc` endpoint,
//! 3. classify call sites with the std-only scanner in this file
//!    (`str::find` + char checks — no regex crate, no `unsafe`),
//! 4. return resolved edges plus an honest `unresolved` bucket — a dynamic
//!    call site is never silently dropped.
//!
//! Every interpolated name passes a `names.rs` validator BEFORE any HTTP
//! request, so a refusal means zero calls on the wire.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::iris::IrisClient;
use crate::iris::documents as docapi;
use crate::iris::sql as sqlapi;
use crate::tools::names::{
    valid_class_name, valid_member_name, valid_namespace, valid_routine_name,
};
use crate::tools::search::{SearchCodeArgs, search_code};

/// Candidate-doc ceiling for one narrowing search.
const SEARCH_DOC_LIMIT: u32 = 500;

/// Fetch concurrency for the parse-on-fetch stage.
const FETCH_CONCURRENCY: usize = 8;

/// Cap on the macro-definition fan-out per call (latency gate).
const MACRO_LOOKUP_LIMIT: usize = 32;

/// How a call site was found (wire strings).
mod via {
    /// `##class(Target).Member(...)` literal call site.
    pub const CLASS_CALL: &str = "class_call";
    /// `..Member(...)` inside the target class itself.
    pub const SELF_CALL: &str = "self_call";
    /// `Do/Goto ^Routine` or `^Routine(` (routine targets).
    pub const ROUTINE_DO: &str = "routine_do";
    /// Exact `Super` dictionary row (class targets).
    pub const SUPER: &str = "super";
    /// A property's `SqlComputeCode` carries the call.
    pub const COMPUTED: &str = "computed";
    /// A trigger body carries the call.
    pub const TRIGGER: &str = "trigger";
}

/// Why the scanner cannot resolve a call site (wire strings).
mod hint {
    /// `o.Member(...)` — receiver type not known statically.
    pub const OREF: &str = "oref_call";
    /// The name appears inside a string literal (eval'd at runtime).
    pub const DYNAMIC_STRING: &str = "dynamic_string";
    /// Reached through a `$$$MACRO` whose expansion carries the target.
    pub const MACRO: &str = "macro";
}

/// A parsed, guarded reference target: exactly
/// `"Package.Class.Member" | "Package.Class" | "^Routine"`. The member
/// suffix splits at the LAST dot only when both halves are independently
/// valid names; the leading `^` of the routine form is stripped BEFORE the
/// name guard, so a caret never reaches an interpolated position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    /// A class.
    Class(String),
    /// A member of a class.
    Member {
        /// Owning class.
        class: String,
        /// Member name.
        member: String,
    },
    /// A routine (caret stripped).
    Routine(String),
}

/// Parse + guard the target grammar BEFORE any HTTP.
///
/// # Errors
/// [`Error::Config`] on an empty, malformed, or unsafe target.
pub fn parse_ref_target(raw: &str) -> Result<RefTarget> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::Config(
            "target is empty (expected Class.Member, Class, or ^Routine)".to_string(),
        ));
    }
    if let Some(rest) = raw.strip_prefix('^') {
        if rest.is_empty() || rest.contains('^') || !valid_routine_name(rest) {
            return Err(Error::Config(format!(
                "invalid routine name (letters/digits/_/-/., 1-255 chars, no caret): {rest}"
            )));
        }
        return Ok(RefTarget::Routine(rest.to_string()));
    }
    if raw.contains('^') {
        return Err(Error::Config(format!(
            "invalid target: a caret may only lead a ^Routine name: {raw}"
        )));
    }
    // Try the `Class.Member` split at the LAST dot whenever both halves are
    // independently valid names; a 2-element "Package.Class" ALSO splits
    // (left = "Package", a legal root-package class name) — the dictionary
    // existence check below re-reads it as a class name when the left half
    // has no row and the whole string does (G_SPEC §2's "attempt the
    // member-less reading" rule, resolved against %Dictionary truth).
    match raw.rfind('.') {
        Some(i)
            if i > 0
                && i + 1 < raw.len()
                && valid_class_path(&raw[..i])
                && valid_member_name(&raw[i + 1..]) =>
        {
            Ok(RefTarget::Member {
                class: raw[..i].to_string(),
                member: raw[i + 1..].to_string(),
            })
        }
        _ => {
            if !valid_class_path(raw) {
                return Err(Error::Config(format!(
                    "invalid target (not a Class, Class.Member, or ^Routine): {raw}"
                )));
            }
            Ok(RefTarget::Class(raw.to_string()))
        }
    }
}

/// Class-path validity for target parsing: like `valid_class_name` but every
/// dot-separated package element must be non-empty and a plain identifier —
/// `A.B.` / `.B` / `A..B` are grammar junk, not class names.
fn valid_class_path(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|el| {
            let mut ch = el.chars();
            match ch.next() {
                Some(c) if c.is_ascii_alphabetic() || c == '%' => {
                    ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
                }
                _ => false,
            }
        })
}

impl RefTarget {
    /// Canonical spelling echoed in the result.
    fn canonical(&self) -> String {
        match self {
            RefTarget::Class(c) => c.clone(),
            RefTarget::Member { class, member } => format!("{class}.{member}"),
            RefTarget::Routine(r) => format!("^{r}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            RefTarget::Class(_) => "class",
            RefTarget::Member { .. } => "class_member",
            RefTarget::Routine(_) => "routine",
        }
    }

    /// The class a target hangs off (`None` for routines).
    fn class(&self) -> Option<&str> {
        match self {
            RefTarget::Class(c) | RefTarget::Member { class: c, .. } => Some(c),
            RefTarget::Routine(_) => None,
        }
    }

    /// Member name for member targets (`None` otherwise).
    fn member(&self) -> Option<&str> {
        match self {
            RefTarget::Member { member, .. } => Some(member),
            _ => None,
        }
    }

    /// The literal needle for candidate narrowing: the member name for
    /// member targets (every call form contains it as literal text), the
    /// class/routine name otherwise.
    fn needle(&self) -> &str {
        match self {
            RefTarget::Member { member, .. } => member,
            RefTarget::Class(c) => c,
            RefTarget::Routine(r) => r,
        }
    }
}

/// Arguments for [`find_references`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindReferencesArgs {
    /// Target: "Package.Class.Member", "Package.Class", or "^`RoutineName`"
    pub target: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Include system documents (default false: a full system scan measured
    /// 3.09 s wall, above the 2 s query gate, so it stays opt-in)
    pub include_system: Option<bool>,
    /// Include generated/compiled .int docs (default false: they carry
    /// macro-expanded call sites but flood the result with generated global
    /// references — a completeness backstop, opt-in)
    pub include_generated: Option<bool>,
    /// Max resolved+unresolved edges (default: settings `search_max_results`)
    pub max_results: Option<u32>,
}

/// One RESOLVED reference: the scanner proved the call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RefEdge {
    /// Source document (`FProbeG.Caller.cls`).
    pub doc: String,
    /// Containing member when attributable.
    pub member: Option<String>,
    /// 1-based line within the fetched document.
    pub line: Option<u32>,
    /// The source line, trimmed, at most 200 chars.
    pub text: String,
    /// `class_call` | `self_call` | `routine_do` | `super` | `computed` | `trigger`.
    pub via: String,
}

/// One dynamic call site that cannot be resolved statically — reported,
/// never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnresolvedRef {
    /// Source document.
    pub doc: String,
    /// Containing member when attributable.
    pub member: Option<String>,
    /// 1-based line.
    pub line: Option<u32>,
    /// The source line, trimmed, at most 200 chars.
    pub text: String,
    /// `oref_call` | `dynamic_string` | `macro`.
    pub hint: String,
}

/// Result of [`find_references`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FindReferencesResult {
    /// Namespace scanned.
    pub namespace: String,
    /// Canonicalized target.
    pub target: String,
    /// `class` | `class_member` | `routine`.
    pub kind: String,
    /// Resolved edges (scanner-proven).
    pub edges: Vec<RefEdge>,
    /// Dynamic forms with hints.
    pub unresolved: Vec<UnresolvedRef>,
    /// Documents fetched and scanned (SQL-derived rows are not scans).
    pub docs_scanned: usize,
    /// True when `max_results` cut the output or a narrowing search hit its
    /// own limit.
    pub truncated: bool,
}

/// Trim to 200 chars (code-point safe).
fn clip(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 200 {
        return t.to_string();
    }
    let mut out: String = t.chars().take(200).collect();
    out.push('…');
    out
}

/// True when no identifier byte sits immediately before `pos`.
fn boundary_before(hay: &str, pos: usize) -> bool {
    match hay.as_bytes().get(pos.wrapping_sub(1)) {
        None => true,
        Some(&c) => !(c.is_ascii_alphanumeric() || c == b'_' || c == b'%' || c == b'$'),
    }
}

/// True when no identifier byte sits at `pos` (the byte AFTER a token).
fn boundary_at(hay: &str, pos: usize) -> bool {
    match hay.as_bytes().get(pos) {
        None => true,
        Some(&c) => !(c.is_ascii_alphanumeric() || c == b'_' || c == b'%'),
    }
}

/// Byte offset of the first `//` line comment in `line` that is NOT inside a
/// double-quoted string (doubled `""` counts as an escaped quote). A `///`
/// doc-comment line returns 0 — doc lines never produce edges.
fn comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                if in_str && b.get(i + 1) == Some(&b'"') {
                    i += 1;
                } else {
                    in_str = !in_str;
                }
            }
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// True when byte `col` of `line` sits inside a double-quoted string.
fn in_string_at(line: &str, col: usize) -> bool {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0usize;
    while i < col.min(b.len()) {
        if b[i] == b'"' {
            if in_str && b.get(i + 1) == Some(&b'"') {
                i += 1;
            } else {
                in_str = !in_str;
            }
        }
        i += 1;
    }
    in_str
}

/// Read an identifier-ish token (letters/digits/`_`/`%`/`.`/`-`) at `i`;
/// returns (end, token).
fn read_token(hay: &str, i: usize) -> (usize, String) {
    let b = hay.as_bytes();
    let mut j = i;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'%' | b'.' | b'-'))
    {
        j += 1;
    }
    (j, hay[i..j].to_string())
}

/// The identifier-ish token immediately before offset `i` (walking left).
fn token_before(hay: &str, i: usize) -> (usize, String) {
    let b = hay.as_bytes();
    let mut j = i;
    while j > 0
        && (b[j - 1].is_ascii_alphanumeric() || matches!(b[j - 1], b'_' | b'%' | b'.' | b'-'))
    {
        j -= 1;
    }
    (j, hay[j..i].to_string())
}

/// Skip spaces/tabs/newlines.
fn skip_ws_soft(hay: &str, mut i: usize) -> usize {
    while matches!(
        hay.as_bytes().get(i),
        Some(&b' ' | &b'\t' | &b'\n' | &b'\r')
    ) {
        i += 1;
    }
    i
}

/// Walk left from a closing paren to its match; true when `##class` (any
/// case) sits directly before that opening paren.
fn opens_class_call(hay: &str, close: usize) -> bool {
    let b = hay.as_bytes();
    let mut depth = 1i32;
    let mut i = close;
    while i > 0 && depth > 0 {
        i -= 1;
        match b[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 || b.get(i) != Some(&b'(') {
        return false;
    }
    let mut start = i.saturating_sub(16);
    while start < i && !hay.is_char_boundary(start) {
        start += 1;
    }
    hay[start..i].ends_with("##class") || hay[start..i].ends_with("##CLASS")
}

/// Byte offset of the end of the line containing `pos`.
fn line_end_of(hay: &str, pos: usize) -> usize {
    hay[pos..].find('\n').map_or(hay.len(), |i| pos + i)
}

/// Line-indexed document: the scanner works on the joined text (multi-line
/// call shapes) and maps hits back to (line, column).
struct DocText {
    lines: Vec<String>,
    joined: String,
    starts: Vec<usize>,
    lower: String,
}

impl DocText {
    fn new(lines: &[String]) -> Self {
        let joined = lines.join("\n");
        let mut starts = Vec::with_capacity(lines.len());
        let mut acc = 0usize;
        for l in lines {
            starts.push(acc);
            acc += l.len() + 1;
        }
        Self {
            lower: joined.to_ascii_lowercase(),
            starts,
            lines: lines.to_vec(),
            joined,
        }
    }

    /// (0-based line, column) for a byte offset.
    fn locate(&self, pos: usize) -> (usize, usize) {
        let li = match self.starts.binary_search(&pos) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        (li, pos - self.starts[li])
    }

    /// Where a byte offset sits: `"code"`, `"comment"`, or `"string"`.
    fn region(&self, pos: usize) -> &'static str {
        let (li, col) = self.locate(pos);
        let line = self.lines[li].as_str();
        if let Some(cs) = comment_start(line) {
            if col >= cs {
                return "comment";
            }
        }
        if in_string_at(line, col) {
            return "string";
        }
        "code"
    }
}

/// Per-document attribution context: the UDL member heading each line sits
/// in, plus which lines are inside a `Trigger` body block.
struct MemberTracker {
    /// Member name per line index (`None` = class level).
    names: Vec<Option<String>>,
    /// True for lines inside a trigger body.
    in_trigger_body: Vec<bool>,
}

fn header_member(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = [
        "ClassMethod ",
        "Method ",
        "Trigger ",
        "Query ",
        "Property ",
        "Parameter ",
        "XData ",
    ]
    .iter()
    .find_map(|kw| t.strip_prefix(kw))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '%')
        .collect();
    (!name.is_empty()).then_some(name)
}

impl MemberTracker {
    fn build(lines: &[String]) -> Self {
        let mut names = vec![None; lines.len()];
        let mut in_trigger_body = vec![false; lines.len()];
        let mut current: Option<String> = None;
        // (brace depth inside the trigger block, seen-an-open-brace-yet)
        let mut trig: Option<(i32, bool)> = None;
        for (i, l) in lines.iter().enumerate() {
            let t = l.trim_start();
            if let Some(n) = header_member(l) {
                current = Some(n);
                trig = if t.starts_with("Trigger ") {
                    Some((0, false))
                } else {
                    None
                };
            } else if let Some((d, opened)) = trig {
                let delta = l.chars().filter(|c| *c == '{').count() as i32
                    - l.chars().filter(|c| *c == '}').count() as i32;
                let inside = opened || d > 0 || delta > 0;
                in_trigger_body[i] = inside;
                let nd = d + delta;
                trig = if nd > 0 {
                    Some((nd, true))
                } else if inside {
                    None // block closed on this line
                } else {
                    Some((0, false))
                };
            }
            names[i] = current.clone();
        }
        Self {
            names,
            in_trigger_body,
        }
    }
}

/// One classified hit.
#[derive(Debug)]
struct Hit {
    pos: usize,
    resolved: bool,
    via: &'static str,
    hint: &'static str,
}

/// Scanner context for one document.
struct ScanCtx<'a> {
    target: &'a RefTarget,
    /// Class the doc's own source belongs to (`.cls` stem; `None` for
    /// routines/macros).
    owner_class: Option<String>,
    /// The owner class extends the target class (a `..Member` call in a
    /// subclass body reaches the inherited member — resolved per §2).
    owner_extends_target: bool,
}

/// Classify every call-site shape in one fetched doc; returns hits in byte
/// order, deduped by position.
fn scan_doc(text: &DocText, ctx: &ScanCtx, tracker: &MemberTracker) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    let hay = text.joined.as_str();
    match ctx.target {
        RefTarget::Routine(r) => scan_routine(hay, text, r, &mut hits),
        RefTarget::Class(_) | RefTarget::Member { .. } => {
            scan_class(hay, text, ctx, tracker, &mut hits);
        }
    }
    hits.sort_by_key(|h| h.pos);
    hits
}

/// Routine-target edges: `Do/Goto/^Name(`/`$zobj(^Name` are resolved; a bare
/// same-name global reference is NOT an edge (globals and routines share a
/// namespace — precision rule). String-carried forms → unresolved.
fn scan_routine(hay: &str, text: &DocText, r: &str, hits: &mut Vec<Hit>) {
    let caret = format!("^{r}");
    let mut from = 0usize;
    while let Some(i) = hay[from..].find(&caret) {
        let pos = from + i;
        from = pos + caret.len();
        if !boundary_at(hay, pos + caret.len()) {
            continue; // ^FProbeG.MidData is not ^FProbeG.Mid
        }
        match text.region(pos) {
            "comment" => continue,
            "string" => {
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::DYNAMIC_STRING,
                });
                continue;
            }
            _ => {}
        }
        let before = hay[..pos].trim_end().to_ascii_lowercase();
        let is_control =
            before.ends_with("do") || before.ends_with("goto") || before.ends_with("zobjmethod");
        // `$zobjmethod(^R…` / `$method(^R…`: the caret sits right after the
        // opening paren of a zobj dispatcher — the routine name is a direct
        // argument, so this is a call edge, not a global read.
        let is_zobj_arg = hay[..pos]
            .rsplit_once('(')
            .is_some_and(|(head, _)| head.to_ascii_lowercase().contains("zobj"));
        let is_call = hay.as_bytes().get(pos + caret.len()) == Some(&b'(');
        if is_control || is_call || is_zobj_arg {
            hits.push(Hit {
                pos,
                resolved: true,
                via: via::ROUTINE_DO,
                hint: "",
            });
        }
    }
}

/// Class and class-member edges.
fn scan_class(
    hay: &str,
    text: &DocText,
    ctx: &ScanCtx,
    tracker: &MemberTracker,
    hits: &mut Vec<Hit>,
) {
    let want_class = ctx.target.class().unwrap_or_default();
    let want_member = ctx.target.member();

    // 1. ##class( ... ) [ . Member ] ( — newline-tolerant inside the parens.
    let mut from = 0usize;
    while let Some(i) = text.lower[from..].find("##class(") {
        let pos = from + i;
        from = pos + 8;
        match text.region(pos) {
            "comment" => continue,
            "string" => {
                // A call built inside a string that reaches an eval-ish form
                // (`"…##class("_cls_")…"` + `do $ZF(-4,1,cmd)`): real at
                // runtime, unresolvable statically — honest unresolved.
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::DYNAMIC_STRING,
                });
                continue;
            }
            _ => {}
        }
        let Some((close, cls)) = parse_paren_class(hay, pos + 8) else {
            continue;
        };
        if !cls.eq_ignore_ascii_case(want_class) {
            continue;
        }
        let after = skip_ws_soft(hay, close + 1);
        let member_ok = match (hay[after..].starts_with('.'), want_member) {
            (true, Some(w)) => {
                let (end2, m) = read_token(hay, skip_ws_soft(hay, after + 1));
                let call = hay.as_bytes().get(skip_ws_soft(hay, end2)) == Some(&b'(');
                m.eq_ignore_ascii_case(w) && call
            }
            (true, None) => true, // class target: any member call points at the class
            (false, Some(_)) => false,
            (false, None) => true,
        };
        if member_ok {
            hits.push(Hit {
                pos,
                resolved: true,
                via: via_for_line(text, tracker, pos),
                hint: "",
            });
        }
    }

    if let Some(member) = want_member {
        // 2. ..Member( — self/inherited. Resolved inside the target
        // class (or a subclass body, where it reaches the INHERITED
        // member); inside any other class it is a reference to that
        // class's own same-named member — oref-ish, unresolved.
        let needle = format!("..{member}");
        let mut from = 0usize;
        while let Some(i) = hay[from..].find(&needle) {
            let pos = from + i;
            from = pos + needle.len();
            match text.region(pos) {
                "comment" => continue,
                "string" => {
                    hits.push(Hit {
                        pos,
                        resolved: false,
                        via: "",
                        hint: hint::DYNAMIC_STRING,
                    });
                    continue;
                }
                _ => {}
            }
            if !boundary_before(hay, pos) {
                continue; // `x..M(` is a property chain, not self-ref
            }
            if hay.as_bytes().get(skip_ws_soft(hay, pos + needle.len())) != Some(&b'(') {
                continue; // `..Property` read, not a call
            }
            let in_owner = ctx
                .owner_class
                .as_deref()
                .is_some_and(|oc| oc.eq_ignore_ascii_case(want_class) || ctx.owner_extends_target);
            if in_owner {
                hits.push(Hit {
                    pos,
                    resolved: true,
                    via: via::SELF_CALL,
                    hint: "",
                });
            } else {
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::OREF,
                });
            }
        }

        // 3. `.Member(` occurrences that are NOT part of `..` or
        // `##class(...).`: static `Class.Member(` form resolves; a
        // receiver expression (`o.M(`, `x.y.M(`, `Foo().M(`) is an
        // oref — unresolved by construction.
        let dot = format!(".{member}");
        let mut from = 0usize;
        while let Some(i) = hay[from..].find(&dot) {
            let pos = from + i;
            from = pos + dot.len();
            if pos > 0 && hay.as_bytes()[pos - 1] == b'.' {
                continue; // `..Member` — pass 2 owns it
            }
            match text.region(pos) {
                "comment" => continue,
                "string" => {
                    hits.push(Hit {
                        pos,
                        resolved: false,
                        via: "",
                        hint: hint::DYNAMIC_STRING,
                    });
                    continue;
                }
                _ => {}
            }
            if !boundary_at(hay, pos + dot.len()) {
                continue; // `.TgtMethodX(` is not `.TgtMethod(`
            }
            if hay.as_bytes().get(skip_ws_soft(hay, pos + dot.len())) != Some(&b'(') {
                continue; // not a call
            }
            let prev = pos.wrapping_sub(1);
            if hay.as_bytes().get(prev) == Some(&b')') {
                if opens_class_call(hay, prev) {
                    continue; // `##class(...)M(` — pass 1 owns it
                }
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::OREF,
                });
                continue;
            }
            let (tstart, tok) = token_before(hay, pos);
            if tok.eq_ignore_ascii_case(want_class) && boundary_before(hay, tstart) {
                hits.push(Hit {
                    pos,
                    resolved: true,
                    via: via_for_line(text, tracker, pos),
                    hint: "",
                });
            } else if !tok.is_empty()
                && tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '%')
                && boundary_before(hay, tstart)
            {
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::OREF,
                });
            }
        }

        // 4. `$zobjmethod(recv, "Member", …)` / `$method` dynamic forms.
        for kw in ["$zobjmethod", "$zobjclassmethod", "$method"] {
            let mut from = 0usize;
            while let Some(i) = text.lower[from..].find(kw) {
                let pos = from + i;
                from = pos + kw.len();
                if text.region(pos) != "code" {
                    continue;
                }
                let rest = &hay[pos..line_end_of(hay, pos)];
                if rest.contains(&format!("\"{member}\""))
                    || rest.contains(&format!("\"{want_class}.{member}\""))
                {
                    hits.push(Hit {
                        pos,
                        resolved: false,
                        via: "",
                        hint: hint::DYNAMIC_STRING,
                    });
                }
            }
        }

        // 5. The member name anywhere inside a string literal (the
        // decoy/eval rule: a quoted occurrence is never a resolved
        // edge; reported unresolved so nothing is silently dropped).
        let mut from = 0usize;
        while let Some(i) = hay[from..].find(member) {
            let pos = from + i;
            from = pos + member.len();
            if !boundary_before(hay, pos) || !boundary_at(hay, pos + member.len()) {
                continue;
            }
            if text.region(pos) == "string" {
                hits.push(Hit {
                    pos,
                    resolved: false,
                    via: "",
                    hint: hint::DYNAMIC_STRING,
                });
            }
        }
    } else {
        // Class target: any `..` inside the class's own docs references
        // INTO the class (member unspecified) — resolved self edges.
        let mut i = 0usize;
        while let Some(p) = find_selfdot(hay, i) {
            i = p + 2;
            if text.region(p) != "code" {
                continue;
            }
            let in_owner = ctx
                .owner_class
                .as_deref()
                .is_some_and(|oc| oc.eq_ignore_ascii_case(want_class) || ctx.owner_extends_target);
            if in_owner {
                hits.push(Hit {
                    pos: p,
                    resolved: true,
                    via: via::SELF_CALL,
                    hint: "",
                });
            }
        }
    }
}

/// `via` label for a resolved call-site line: trigger body / `SqlComputeCode`
/// override the plain class call.
fn via_for_line(text: &DocText, tracker: &MemberTracker, pos: usize) -> &'static str {
    let (li, _) = text.locate(pos);
    if text.lines[li].contains("SqlComputeCode") {
        return via::COMPUTED;
    }
    if tracker.in_trigger_body[li] {
        return via::TRIGGER;
    }
    via::CLASS_CALL
}

/// Parse `Class )` from `i` (after `##class(`); returns
/// `(close_paren_index, class_name)`; spaces AND newlines allowed inside.
fn parse_paren_class(hay: &str, i: usize) -> Option<(usize, String)> {
    let j = skip_ws_soft(hay, i);
    let (k, name) = read_token(hay, j);
    if name.is_empty() {
        return None;
    }
    let k = skip_ws_soft(hay, k);
    if hay.as_bytes().get(k) == Some(&b')') {
        Some((k, name))
    } else {
        None
    }
}

/// Index of the next standalone `..` (not `...`, not `x..`) at/after `from`.
fn find_selfdot(hay: &str, from: usize) -> Option<usize> {
    let b = hay.as_bytes();
    let mut i = from;
    while i + 1 < b.len() {
        if b[i] == b'.' && b[i + 1] == b'.' {
            let left_ok = i == 0
                || (!b[i - 1].is_ascii_alphanumeric() && b[i - 1] != b'%' && b[i - 1] != b'.');
            let right_ok = b.get(i + 2).is_none_or(|c| *c != b'.');
            if left_ok && right_ok {
                return Some(i);
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    None
}

async fn rows(client: &IrisClient, ns: &str, sql: &str) -> Result<Vec<Value>> {
    let outcome = sqlapi::query(client, ns, sql, client.settings().sql_max_rows).await?;
    Ok(outcome.rows)
}

async fn class_exists(client: &IrisClient, ns: &str, class: &str) -> Result<bool> {
    let sql = format!("SELECT Name FROM %Dictionary.ClassDefinition WHERE Name = '{class}'");
    Ok(!rows(client, ns, &sql).await?.is_empty())
}

async fn super_of(client: &IrisClient, ns: &str, class: &str) -> Result<Vec<String>> {
    let sql = format!("SELECT Super FROM %Dictionary.ClassDefinition WHERE Name = '{class}'");
    Ok(rows(client, ns, &sql)
        .await?
        .first()
        .and_then(|r| r.get("Super").and_then(Value::as_str))
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// One SQL narrowing query: class parents carrying `needle` inside an
/// embedded-code column. The needle interpolated into the quoted literal
/// passed the name guards first, so it can only be word chars/dots.
async fn sql_parent_query(client: &IrisClient, ns: &str, sql: &str) -> Result<Vec<String>> {
    Ok(rows(client, ns, sql)
        .await?
        .iter()
        .filter_map(|r| r.get("parent").and_then(Value::as_str))
        .filter(|p| valid_class_name(p))
        .map(str::to_string)
        .collect())
}

/// Class a doc name stems from: `Foo.Bar.cls` → `Foo.Bar`; routines,
/// macros, and compiled member `.int` docs → `None` (`.int` bodies carry
/// macro-expanded `##class` calls but no `..` forms, so class attribution
/// stays off for them and the generated-doc profile stays precise).
fn owner_class_of(doc: &str) -> Option<String> {
    doc.strip_suffix(".cls").map(str::to_string)
}

/// Find every reference to `target` in one namespace. Mechanism and honest
/// limits: module docs + `docs/code-search.md`.
///
/// # Errors
/// [`Error::Config`] on an unsafe/invalid target or namespace (before any
/// HTTP); a class/member target with no `%Dictionary.ClassDefinition` row
/// is an honest not-found (same shape as `class_members`), never an empty
/// success; transport/SQL errors otherwise.
pub async fn find_references(
    client: &IrisClient,
    args: &FindReferencesArgs,
) -> Result<FindReferencesResult> {
    // Existence check (dictionary truth): `A.B` may parse as Member under
    // the last-dot rule while actually being a two-package class name — if
    // the left part has no row and the WHOLE string does, re-read it as a
    // class target (one extra dictionary query, only on that shape).
    let mut target = parse_ref_target(&args.target)?;
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace.clone());
    if !valid_namespace(&ns) {
        return Err(Error::Config(format!(
            "invalid namespace (URL path guard): {ns}"
        )));
    }
    match target.clone() {
        RefTarget::Routine(_) => {}
        RefTarget::Class(c) => {
            if !class_exists(client, &ns, &c).await? {
                return Err(crate::tools::graph::not_found_msg(&c, &ns));
            }
        }
        RefTarget::Member { class, member } => {
            let whole = format!("{class}.{member}");
            let (class_row, whole_row) = futures_util::future::join(
                class_exists(client, &ns, &class),
                class_exists(client, &ns, &whole),
            )
            .await;
            if class_row? {
            } else if valid_class_path(&whole) && whole_row? {
                target = RefTarget::Class(whole);
            } else {
                return Err(crate::tools::graph::not_found_msg(&class, &ns));
            }
        }
    }

    let include_system = args.include_system.unwrap_or(false);
    let include_generated = args.include_generated.unwrap_or(false);
    let max_edges = args
        .max_results
        .unwrap_or(client.settings().search_max_results)
        .max(1);

    let mut edges: Vec<RefEdge> = Vec::new();
    let mut unresolved: Vec<UnresolvedRef> = Vec::new();
    let mut truncated = false;

    // Stage 1a/1b (parallel) — literal `action/search` narrowing on the
    // needle (default masks, which now include `*.inc`; `.int` rides
    // `include_generated`) + the two embedded-code SQL narrowing queries.
    // The `[` scans run server-side and the property one measured ~0.9 s
    // cold, so they overlap the search instead of adding to it.
    let needle = target.needle().to_string();
    let class_target = matches!(target, RefTarget::Class(_));
    let (search_res, prop_res, trig_res) = futures_util::future::join3(
        search_code(
            client,
            &SearchCodeArgs {
                query: needle.clone(),
                namespace: Some(ns.clone()),
                documents: None,
                regex: Some(false),
                include_system: Some(include_system),
                include_generated: Some(include_generated),
                case_sensitive: None,
                whole_word: None,
                max_results: Some(SEARCH_DOC_LIMIT),
            },
        ),
        sql_parent_query(
            client,
            &ns,
            &format!("SELECT DISTINCT parent FROM %Dictionary.CompiledProperty WHERE SqlComputeCode [ '{needle}'"),
        ),
        sql_parent_query(
            client,
            &ns,
            &format!("SELECT DISTINCT parent FROM %Dictionary.CompiledTrigger WHERE Code [ '{needle}'"),
        ),
    )
    .await;
    let search = search_res?;
    if search.truncated {
        truncated = true;
    }
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for d in &search.results {
        candidates.insert(d.doc.clone());
    }
    if let Some(class) = target.class() {
        candidates.insert(format!("{class}.cls")); // the target's own doc:
        // `..`-self and the member definition live there.
        for p in prop_res?.into_iter().chain(trig_res?) {
            candidates.insert(format!("{p}.cls"));
        }
    }
    if class_target {
        let class = target.class().unwrap_or_default();
        // Extends/Super rows are dictionary edges (via=super).
        let sql = format!(
            "SELECT Name, Super FROM %Dictionary.ClassDefinition WHERE Super [ '{class}' ORDER BY Name"
        );
        let outcome = sqlapi::query(client, &ns, &sql, client.settings().sql_max_rows).await?;
        if outcome.has_more {
            truncated = true;
        }
        for row in &outcome.rows {
            let Some(name) = row.get("Name").and_then(Value::as_str) else {
                continue;
            };
            let super_raw = row
                .get("Super")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !super_raw.split(',').map(str::trim).any(|s| s == class) {
                continue; // `[` is substring-based: exact-element guard
            }
            edges.push(RefEdge {
                doc: format!("{name}.cls"),
                member: None,
                line: None,
                text: clip(&format!("Class {name} Extends {super_raw}")),
                via: via::SUPER.to_string(),
            });
        }
    }

    // Stage 2 — fetch each candidate ONCE and scan it.
    struct Fetched {
        doc: String,
        text: DocText,
    }
    let mut fetched: Vec<Fetched> = Vec::new();
    let cand: Vec<String> = candidates.into_iter().collect();
    for chunk in cand.chunks(FETCH_CONCURRENCY) {
        let futs = chunk.iter().map(|d| {
            let ns = ns.clone();
            async move { (d.clone(), docapi::get_doc(client, &ns, d).await) }
        });
        for (name, res) in futures_util::future::join_all(futs).await {
            match res {
                Ok(doc) => fetched.push(Fetched {
                    text: DocText::new(&doc.content),
                    doc: name,
                }),
                Err(Error::DocNotFound { .. }) => {
                    tracing::debug!("candidate doc vanished mid-scan: {name}");
                }
                Err(e) => return Err(e),
            }
        }
    }

    // Inheritance context for `..` attribution (memoized one-row SQL).
    let mut owner_extends: Vec<(String, bool)> = Vec::new();
    for f in &fetched {
        let owner = owner_class_of(&f.doc);
        let extends_target = match (target.class(), &owner) {
            (Some(tc), Some(oc)) if !oc.eq_ignore_ascii_case(tc) => {
                if let Some((_, v)) = owner_extends.iter().find(|(k, _)| k == oc) {
                    *v
                } else {
                    let v = super_of(client, &ns, oc)
                        .await?
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(tc));
                    owner_extends.push((oc.clone(), v));
                    v
                }
            }
            (Some(tc), Some(oc)) => oc.eq_ignore_ascii_case(tc),
            _ => false,
        };
        let tracker = MemberTracker::build(&f.text.lines);
        let ctx = ScanCtx {
            target: &target,
            owner_class: owner,
            owner_extends_target: extends_target,
        };
        for h in scan_doc(&f.text, &ctx, &tracker) {
            let (li, _) = f.text.locate(h.pos);
            let line = f.text.lines[li].clone();
            let member = tracker.names[li].clone();
            if h.resolved {
                edges.push(RefEdge {
                    doc: f.doc.clone(),
                    member,
                    line: Some(li as u32 + 1),
                    text: clip(&line),
                    via: h.via.to_string(),
                });
            } else {
                unresolved.push(UnresolvedRef {
                    doc: f.doc.clone(),
                    member,
                    line: Some(li as u32 + 1),
                    text: clip(&line),
                    hint: h.hint.to_string(),
                });
            }
        }
    }

    // Stage 3 — macro reachability: for each `$$$NAME` used on a scanned
    // code line, locate its definition doc (literal search, `*.mac`/`*.inc`
    // masks); when the expansion carries the target, report the USAGE site
    // as unresolved(macro) naming the expansion owner doc — the call edge
    // runs through the macro, not through literal text at the use site.
    if target.member().is_some() {
        struct MacroUse {
            doc: String,
            line: u32,
            name: String,
        }
        let mut uses: Vec<MacroUse> = Vec::new();
        for f in &fetched {
            let mut from = 0usize;
            while let Some(i) = f.text.joined[from..].find("$$$") {
                let pos = from + i;
                from = pos + 3;
                if f.text.region(pos) != "code" {
                    continue;
                }
                let (_, tok) = read_token(&f.text.joined, pos + 3);
                let clean: String = tok
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '%')
                    .collect();
                if !clean.is_empty() {
                    let (li, _) = f.text.locate(pos);
                    uses.push(MacroUse {
                        doc: f.doc.clone(),
                        line: li as u32 + 1,
                        name: clean,
                    });
                }
            }
        }
        let mut checked: BTreeSet<String> = BTreeSet::new();
        let mut carried: BTreeSet<String> = BTreeSet::new();
        for u in &uses {
            if !checked.insert(u.name.clone()) {
                continue;
            }
            if checked.len() > MACRO_LOOKUP_LIMIT {
                break;
            }
            let ms = search_code(
                client,
                &SearchCodeArgs {
                    // Bare name: the `$%%` prefix is search-engine-hostile
                    // and the define line carries it either way; the
                    // `$$$NAME =` shape filter below keeps only definitions.
                    query: u.name.clone(),
                    namespace: Some(ns.clone()),
                    documents: Some(vec!["*.mac".into(), "*.inc".into()]),
                    regex: Some(false),
                    include_system: Some(include_system),
                    include_generated: Some(false),
                    case_sensitive: None,
                    whole_word: None,
                    max_results: Some(SEARCH_DOC_LIMIT),
                },
            )
            .await?;
            for d in &ms.results {
                // Definition text: the line `Set Definition $$$NAME = {…}`
                // (or the `$$$NAME = {…}` shape). Reuse stage-2 content when
                // the doc was already fetched; otherwise one extra GET.
                let joined = if let Some(f) = fetched.iter().find(|f| f.doc == d.doc) {
                    f.text.joined.clone()
                } else if let Ok(doc) = docapi::get_doc(client, &ns, &d.doc).await {
                    doc.content.join("\n")
                } else {
                    continue;
                };
                let define_line = joined.lines().find(|l| {
                    l.contains(&format!("$$$ {} =", u.name))
                        || l.contains(&format!("$$$ {}=", u.name))
                        || l.contains(&format!("$$${} =", u.name))
                        || l.contains(&format!("$$${}=", u.name))
                });
                let Some(define_line) = define_line else {
                    continue;
                };
                // Does the expansion carry the target (name or class as
                // literal text outside a trailing comment)?
                let code = match comment_start(define_line) {
                    Some(cs) => &define_line[..cs],
                    None => define_line,
                };
                let carries = code.contains(target.needle())
                    || target.class().is_some_and(|c| code.contains(c));
                if carries {
                    carried.insert(u.name.clone());
                    break;
                }
            }
        }
        // Second pass: emit at every use site of a carried macro.
        for u in &uses {
            if carried.contains(&u.name) {
                unresolved.push(UnresolvedRef {
                    doc: u.doc.clone(),
                    member: None,
                    line: Some(u.line),
                    text: clip(&format!("$$$ {}", u.name)),
                    hint: hint::MACRO.to_string(),
                });
            }
        }
    }

    // Collapse duplicate unresolved reports for the same source line (a
    // string-carried name can match several passes).
    let mut seen: BTreeSet<(String, Option<u32>, String)> = BTreeSet::new();
    unresolved.retain(|u| seen.insert((u.doc.clone(), u.line, u.hint.clone())));

    edges.sort_by(|a, b| {
        (&a.doc, a.line.unwrap_or(u32::MAX), &a.via).cmp(&(
            &b.doc,
            b.line.unwrap_or(u32::MAX),
            &b.via,
        ))
    });
    unresolved.sort_by(|a, b| {
        (&a.doc, a.line.unwrap_or(u32::MAX)).cmp(&(&b.doc, b.line.unwrap_or(u32::MAX)))
    });

    let total = edges.len() + unresolved.len();
    if total > max_edges as usize {
        truncated = true;
        let keep = max_edges as usize;
        let etake = edges.len().min(keep);
        edges.truncate(etake);
        unresolved.truncate(keep - etake);
    }

    Ok(FindReferencesResult {
        namespace: ns,
        target: target.canonical(),
        kind: target.kind().to_string(),
        edges,
        unresolved,
        docs_scanned: fetched.len(),
        truncated,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(lines: &[&str]) -> DocText {
        DocText::new(
            &lines
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>(),
        )
    }

    fn member_target() -> RefTarget {
        RefTarget::Member {
            class: "FProbeG.Target".into(),
            member: "TgtMethod".into(),
        }
    }

    fn ctx_for<'a>(t: &'a RefTarget, owner: Option<&'a str>, extends: bool) -> ScanCtx<'a> {
        ScanCtx {
            target: t,
            owner_class: owner.map(str::to_string),
            owner_extends_target: extends,
        }
    }

    fn scan(t: &RefTarget, owner: Option<&str>, extends: bool, d: &DocText) -> Vec<Hit> {
        let tr = MemberTracker::build(&d.lines);
        scan_doc(d, &ctx_for(t, owner, extends), &tr)
    }

    /// U16 — target grammar: member split at the LAST dot, caret stripped
    /// before the guard, junk refused without ever interpolating.
    #[test]
    fn target_grammar() {
        assert_eq!(
            parse_ref_target("FProbeG.Target.TgtMethod").unwrap(),
            member_target()
        );
        assert_eq!(
            parse_ref_target(" ^FProbeG.Mid ").unwrap(),
            RefTarget::Routine("FProbeG.Mid".into())
        );
        assert_eq!(
            parse_ref_target("%Zombie").unwrap(),
            RefTarget::Class("%Zombie".into())
        );
        assert_eq!(
            parse_ref_target("%Library.RegisteredObject.%New").unwrap(),
            RefTarget::Member {
                class: "%Library.RegisteredObject".into(),
                member: "%New".into(),
            }
        );
        // Trailing dot / leading dot are grammar junk.
        assert!(parse_ref_target("A.B.").is_err());
        assert!(parse_ref_target(".B").is_err());
        // A 2-element name: the last-dot rule splits it as root-package
        // Class.Member ("FProbeG"."Target"); the runtime dictionary check
        // re-reads it as a class name when only the whole string has a row
        // (pinned against wiremock below, and by the live recall suite).
        assert_eq!(
            parse_ref_target("FProbeG.Target").unwrap(),
            RefTarget::Member {
                class: "FProbeG".into(),
                member: "Target".into(),
            }
        );
        assert_eq!(
            parse_ref_target("MyClass.MyMethod").unwrap(),
            RefTarget::Member {
                class: "MyClass".into(),
                member: "MyMethod".into(),
            }
        );
        for junk in [
            "",
            "   ",
            "^Bad';Name",
            "^",
            "Robert'; DROP TABLE",
            "Class'Name",
            "^two^carets",
            "mid^caret.Class",
            "1StartsDigit",
            "Package.Class.Bad Member",
        ] {
            assert!(parse_ref_target(junk).is_err(), "must refuse {junk:?}");
        }
        assert!(!valid_routine_name(&"x".repeat(256)));
        assert!(valid_routine_name(&"x".repeat(255)));
    }

    /// E1 + E4 + E5 fixture shapes: one resolved ##class edge, dynamic
    /// forms unresolved, comment lines never hit.
    #[test]
    fn scanner_class_call_dynamic_and_comment() {
        let t = member_target();
        let d = doc(&[
            "ClassMethod Edge1() As %Status",
            "{",
            "    set t = ##class(FProbeG.Target).TgtMethod(1)",
            "    set r = $zobjmethod(obj, \"TgtMethod\", 4)",
            "    set cmd = \"s x=##class(\"_cls_\").TgtMethod(5)\"",
            "    // ##class(FProbeG.Target).TgtMethod(99)",
            "    quit",
            "}",
        ]);
        let hits = scan(&t, Some("FProbeG.Caller"), false, &d);
        let resolved: Vec<_> = hits.iter().filter(|h| h.resolved).collect();
        assert_eq!(resolved.len(), 1, "resolved hits: {resolved:?}");
        assert_eq!(resolved[0].via, via::CLASS_CALL);
        assert_eq!(d.locate(resolved[0].pos).0, 2);
        // Unresolved lines: the zobjmethod line + the assembled-string line;
        // the comment line NEVER appears anywhere.
        let lines: BTreeSet<usize> = hits
            .iter()
            .filter(|h| !h.resolved)
            .map(|h| d.locate(h.pos).0)
            .collect();
        assert_eq!(lines, BTreeSet::from([3, 4]), "unresolved lines: {lines:?}");
        assert!(
            hits.iter()
                .filter(|h| !h.resolved)
                .all(|h| h.hint == hint::DYNAMIC_STRING)
        );
        assert!(
            !hits
                .iter()
                .any(|h| d.lines[d.locate(h.pos).0].contains("// ##class"))
        );
    }

    /// E6 self-call inside the class resolves; the prefix-collision decoy
    /// (`TgtMethodX`) never hits; the bare word in a string is unresolved
    /// (`dynamic_string`) — zero resolved decoy edges.
    #[test]
    fn scanner_self_prefix_and_string() {
        let t = member_target();
        let d = doc(&[
            "Method MInst() As %Integer",
            "{",
            "    Quit ..TgtMethod(6)",
            "    set v = x.TgtMethodX(2)",
            "    write \"TgtMethod is the name we are hunting\",!",
            "}",
        ]);
        let hits = scan(&t, Some("FProbeG.Target"), true, &d);
        let resolved: Vec<_> = hits.iter().filter(|h| h.resolved).collect();
        assert_eq!(resolved.len(), 1, "{resolved:?}");
        assert_eq!(resolved[0].via, via::SELF_CALL);
        assert_eq!(d.locate(resolved[0].pos).0, 2);
        let un: Vec<_> = hits.iter().filter(|h| !h.resolved).collect();
        assert_eq!(un.len(), 1, "{un:?}");
        assert_eq!(un[0].hint, hint::DYNAMIC_STRING);
        assert_eq!(d.locate(un[0].pos).0, 4);
    }

    /// E7 (trigger body) → via=trigger; E8 (`SqlComputeCode`) → via=computed.
    #[test]
    fn scanner_trigger_and_computed() {
        let t = member_target();
        let d = doc(&[
            "Trigger trg1 [ Event = INSERT ]",
            "{",
            " write ##class(FProbeG.Target).TgtMethod(7),!",
            "}",
            "Property P9 As %Integer [ SqlComputeCode = {Set {*} = ##class(FProbeG.Target).TgtMethod(8)}, SqlComputed];",
        ]);
        let hits = scan(&t, Some("FProbeG.Target"), true, &d);
        let resolved: Vec<_> = hits
            .iter()
            .filter(|h| h.resolved)
            .map(|h| (d.locate(h.pos).0, h.via))
            .collect();
        assert_eq!(resolved.len(), 2, "{resolved:?}");
        assert!(resolved.contains(&(2, via::TRIGGER)), "{resolved:?}");
        assert!(resolved.contains(&(4, via::COMPUTED)), "{resolved:?}");
    }

    /// E10: `o.TgtMethod(` is an oref form — `unresolved(oref_call)`.
    #[test]
    fn scanner_oref_call() {
        let t = member_target();
        let d = doc(&["    set r = o.TgtMethod(10)"]);
        let hits = scan(&t, Some("FProbeG.Sub"), false, &d);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(!hits[0].resolved);
        assert_eq!(hits[0].hint, hint::OREF);
    }

    /// Static qualified `FProbeG.Target.TgtMethod(` resolves as a class
    /// call (the class part spells the target).
    #[test]
    fn scanner_static_qualified() {
        let t = member_target();
        let d = doc(&["    set q = FProbeG.Target.TgtMethod(1)"]);
        let hits = scan(&t, Some("FProbeG.Caller"), false, &d);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].resolved);
        assert_eq!(hits[0].via, via::CLASS_CALL);
    }

    /// E3: `Do ^FProbeG.Mid` resolves; the name-prefixed global does NOT;
    /// the `^Name(` subscripted form resolves (routine-call syntax).
    #[test]
    fn scanner_routine_do_not_prefixed_global() {
        let t = RefTarget::Routine("FProbeG.Mid".into());
        let d = doc(&[
            "FProbeG.EdgeR",
            "\tDo ^FProbeG.Mid",
            "\tset g = ^FProbeG.MidData(1)",
            "\tset h = ^FProbeG.Mid(1)",
            "\tset i = \"Do ^FProbeG.Mid\"",
        ]);
        let hits = scan(&t, None, false, &d);
        let got: Vec<_> = hits
            .iter()
            .map(|h| (d.locate(h.pos).0, h.resolved, h.via, h.hint))
            .collect();
        assert_eq!(got[0], (1, true, via::ROUTINE_DO, ""), "{got:?}");
        assert_eq!(got[1], (3, true, via::ROUTINE_DO, ""), "{got:?}");
        assert_eq!(got[2], (4, false, "", hint::DYNAMIC_STRING), "{got:?}");
        assert_eq!(got.len(), 3, "{got:?}");
    }

    /// Multi-line `##class(` shape resolves once (no oref double-report).
    #[test]
    fn scanner_multiline() {
        let t = member_target();
        let d = doc(&[
            "set r = ##class(",
            "      FProbeG.Target",
            "   ).TgtMethod(",
            "      1)",
        ]);
        let hits = scan(&t, Some("FProbeG.Caller"), false, &d);
        assert_eq!(
            hits.len(),
            1,
            "{:?}",
            hits.iter().map(|h| (h.pos, h.resolved)).collect::<Vec<_>>()
        );
        assert!(hits[0].resolved);
        assert_eq!(d.locate(hits[0].pos).0, 0);
    }

    /// Class target: ##`class(Foo).%New()` and `..`-self inside Foo both
    /// resolve.
    #[test]
    fn scanner_class_target() {
        let t = RefTarget::Class("FProbeG.Target".into());
        let d = doc(&[
            "    set v = ##class(FProbeG.Target).%New()",
            "    Quit ..TgtMethod(6)",
        ]);
        let hits = scan(&t, Some("FProbeG.Target"), true, &d);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.resolved));
    }

    /// `///` doc-comment lines never produce edges.
    #[test]
    fn scanner_doc_comment_excluded() {
        let t = member_target();
        let d = doc(&["/// calls ##class(FProbeG.Target).TgtMethod(0) for demo"]);
        let hits = scan(&t, Some("FProbeG.Caller"), false, &d);
        assert!(hits.is_empty(), "{hits:?}");
    }

    /// U2 — serde contract for the new args struct.
    #[test]
    fn refs_args_denies_unknown() {
        let a: FindReferencesArgs = serde_json::from_value(json!({
            "target": "FProbeG.Target.TgtMethod",
            "include_system": false,
            "include_generated": false,
            "max_results": 10,
        }))
        .unwrap();
        assert_eq!(a.target, "FProbeG.Target.TgtMethod");
        assert!(a.namespace.is_none());
        assert!(
            serde_json::from_value::<FindReferencesArgs>(json!({"target":"A.B","nope":1})).is_err()
        );
        assert!(serde_json::from_value::<FindReferencesArgs>(json!({})).is_err());
    }

    #[test]
    fn doctext_locate_and_region() {
        let d = doc(&["abc", r#"write "a//b" // real"#, "\tDo ^R"]);
        assert_eq!(d.locate(0), (0, 0));
        assert_eq!(d.locate(4), (1, 0));
        assert_eq!(d.locate(25), (2, 0));
        assert_eq!(d.region(0), "code");
        // inside the "a//b" string
        assert_eq!(d.region(4 + 8), "string");
        // after the closing quote: the real comment
        assert_eq!(d.region(4 + 15), "comment");
    }

    #[test]
    fn line_comment_states() {
        assert_eq!(comment_start(r#"write "a//b""#), None);
        assert_eq!(comment_start("write 1 // a//b"), Some(8));
        assert_eq!(comment_start("/// doc line"), Some(0));
        assert_eq!(comment_start(r#"set x="he said ""hi//x"""#), None);
    }

    #[test]
    fn owner_class_of_stems() {
        assert_eq!(
            owner_class_of("FProbeG.Target.cls").as_deref(),
            Some("FProbeG.Target")
        );
        assert_eq!(owner_class_of("FProbeG.EdgeR.mac"), None);
        assert_eq!(owner_class_of("FProbeG.Target.1.int"), None);
    }

    #[test]
    fn member_tracker_attributes_lines() {
        let d = doc(&[
            "ClassMethod A() As %Status",
            "{",
            "\twrite 1",
            "}",
            "Trigger trg1 [ Event = INSERT ]",
            "{",
            "\twrite 2",
            "}",
            "Method B()",
            "{",
            "}",
        ]);
        let tr = MemberTracker::build(&d.lines);
        assert_eq!(tr.names[2].as_deref(), Some("A"));
        assert_eq!(tr.names[6].as_deref(), Some("trg1"));
        assert!(tr.in_trigger_body[6], "trigger body line");
        assert!(!tr.in_trigger_body[2]);
        assert_eq!(tr.names[9].as_deref(), Some("B"));
        assert!(!tr.in_trigger_body[9]);
    }

    #[test]
    fn opens_class_call_detects_paren_origin() {
        let hay = "set v = ##class(Foo.Bar).Baz(".to_string();
        let dot = hay.find(".Baz").unwrap();
        assert!(opens_class_call(&hay, dot - 1));
        let hay2 = "set v = ..Factory().Baz(".to_string();
        let dot2 = hay2.find(".Baz").unwrap();
        assert!(!opens_class_call(&hay2, dot2 - 1));
    }
}
