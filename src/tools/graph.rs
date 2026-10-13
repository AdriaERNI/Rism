//! Reference-graph tools over `%Dictionary` SQL (issue #17, Tier 2):
//! `find_subclasses`, `class_members`, `class_dependencies`. Framework-free
//! (no rmcp/clap types). The endpoint has NO bind parameters, so every
//! class name interpolated into SQL passes [`valid_class_name`] BEFORE any
//! HTTP request is made — refusal means zero calls (`E_SPEC` §3).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::iris::IrisClient;
use crate::iris::sql as sqlapi;
use crate::tools::names::{valid_class_name, valid_identifier};

/// Query budget per tool call across a whole BFS walk (`E_SPEC` §3): more
/// frontier queries than this sets `truncated` instead of fanning out.
const FRONTIER_QUERY_BUDGET: usize = 256;

/// Hard ceiling on `depth` arguments (values above clamp, never error).
const MAX_DEPTH: u32 = 64;

fn ns_of(client: &IrisClient, ns: Option<&String>) -> String {
    ns.cloned()
        .unwrap_or_else(|| client.settings().iris_namespace.clone())
}

/// Guard every class-name ingress: SQL is string-built (no binds), so an
/// unsafe name is a hard [`Error::Config`] BEFORE any HTTP request.
fn guard(class: &str) -> Result<()> {
    if !valid_class_name(class) {
        return Err(Error::Config(format!(
            "invalid class name (interpolation guard): {class}"
        )));
    }
    Ok(())
}

/// One edge of a subclass/dependency walk: the found name, the raw field
/// that produced the edge, and its BFS distance from the start class.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GraphNode {
    /// Class name found.
    pub name: String,
    /// Parent (subclasses) or depended-on/depending class (dependencies).
    /// `super` is a Rust keyword; serde pins the frozen wire name.
    #[serde(rename = "super")]
    pub parent: String,
    /// 1 = direct child/dependency of the start class.
    pub level: u32,
}

/// Arguments for [`find_subclasses`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindSubclassesArgs {
    /// Class to start from (must exist; result excludes it)
    pub class: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Max BFS levels (default: settings `graph_depth`; 0 is an error, >64 clamps)
    pub depth: Option<u32>,
}

/// Result of [`find_subclasses`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FindSubclassesResult {
    /// Start class.
    pub class: String,
    /// Effective BFS depth limit.
    pub depth: u32,
    /// Number of distinct subclasses found.
    pub count: usize,
    /// True when the walk ended with a non-empty frontier or hit the
    /// 256-query budget.
    pub truncated: bool,
    /// Subclasses, BFS order (level, then name).
    pub subclasses: Vec<GraphNode>,
}

/// Result of [`find_subclasses`] — bounded BFS over `Super [ 'F'` edges.
///
/// # Errors
/// [`Error::Config`] on unsafe class name or `depth = 0`; transport/SQL
/// errors from `action/query`.
pub async fn find_subclasses(
    client: &IrisClient,
    args: &FindSubclassesArgs,
) -> Result<FindSubclassesResult> {
    guard(&args.class)?;
    let ns = ns_of(client, args.namespace.as_ref());
    let depth = resolve_depth(args.depth, client.settings().graph_depth)?;

    let mut visited: Vec<String> = vec![args.class.clone()];
    let mut found: Vec<GraphNode> = Vec::new();
    let mut frontier: Vec<String> = vec![args.class.clone()];
    let mut queries = 0usize;
    let mut truncated = false;
    let mut exhausted = false;

    'levels: for level in 1..=depth {
        let mut next: Vec<String> = Vec::new();
        for f in &frontier {
            // Server-sourced frontier names are NOT trusted: re-guard.
            if !valid_class_name(f) {
                tracing::warn!("dropping unsafe frontier class name: {f}");
                continue;
            }
            if queries >= FRONTIER_QUERY_BUDGET {
                truncated = true;
                break 'levels;
            }
            queries += 1;
            let sql = format!(
                "SELECT Name, Super FROM %Dictionary.ClassDefinition WHERE Super [ '{f}' ORDER BY Name"
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
                // `[` is substring-based: require an EXACT superclass edge.
                if !super_split_exact(&super_raw, f) {
                    continue;
                }
                if visited.iter().any(|v| v == name) {
                    continue;
                }
                visited.push(name.to_string());
                next.push(name.to_string());
                found.push(GraphNode {
                    name: name.to_string(),
                    parent: super_raw,
                    level,
                    // (level is already u32)
                });
            }
        }
        if next.is_empty() {
            exhausted = true;
            break;
        }
        frontier = next;
    }
    // Loop ended with a live frontier and the walk was NOT budget-cut:
    // deeper levels exist that we did not visit → truncated (E_SPEC §3).
    if !truncated && !exhausted && !frontier.is_empty() {
        truncated = true;
    }

    Ok(FindSubclassesResult {
        class: args.class.clone(),
        depth,
        count: found.len(),
        truncated,
        subclasses: found,
    })
}

/// Member-kind selector for [`class_members`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MembersKind {
    /// All four member tables.
    #[default]
    All,
    /// Methods only.
    Methods,
    /// Properties only.
    Properties,
    /// Parameters only.
    Parameters,
    /// `XData` blocks only (names — the stream data is never selected).
    #[serde(rename = "xdata")] // frozen wire string (§4: `xdata`, not `x_data`)
    XData,
}

/// Arguments for [`class_members`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassMembersArgs {
    /// Class to inspect
    pub class: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Which member tables to read (default all)
    pub kind: Option<MembersKind>,
    /// Include inherited members (default false)
    pub include_inherited: Option<bool>,
}

/// One method row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberMethod {
    /// Method name.
    pub name: String,
    /// `ClassMethod` flag.
    pub class_method: bool,
    /// `SqlProc` flag.
    pub sql_proc: bool,
    /// Formal spec string.
    pub formal_spec: String,
    /// Return type (often empty).
    pub return_type: String,
    /// Declaring class.
    pub origin: String,
}

/// One property row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberProperty {
    /// Property name.
    pub name: String,
    /// Type, fully qualified (e.g. `%Library.String`).
    pub type_: String,
    /// Collection kind ("" when scalar).
    pub collection: String,
    /// Declaring class.
    pub origin: String,
}

/// One parameter row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberParameter {
    /// Parameter name.
    pub name: String,
    /// Type when the dictionary reports one (often empty -> null).
    pub type_: Option<String>,
    /// Declaring class.
    pub origin: String,
}

/// One `XData` block (name only — its stream is never queried, issue #17).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberXData {
    /// `XData` block name.
    pub name: String,
}

/// Member counts per table.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberCounts {
    /// Methods returned.
    pub methods: usize,
    /// Properties returned.
    pub properties: usize,
    /// Parameters returned.
    pub parameters: usize,
    /// `XData` blocks returned.
    pub xdata: usize,
}

/// Result of [`class_members`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClassMembersResult {
    /// Inspected class.
    pub class: String,
    /// Requested kind (`"all"` when absent).
    pub kind: String,
    /// Whether inherited members were included.
    pub include_inherited: bool,
    /// Counts per table (0 for kinds not queried).
    pub counts: MemberCounts,
    /// Methods (empty when not requested).
    pub methods: Vec<MemberMethod>,
    /// Properties (empty when not requested).
    pub properties: Vec<MemberProperty>,
    /// Parameters (empty when not requested).
    pub parameters: Vec<MemberParameter>,
    /// `XData` names (empty when not requested).
    pub xdata: Vec<MemberXData>,
}

/// List a class's compiled members from `%Dictionary.Compiled*`.
///
/// # Errors
/// [`Error::Config`] on unsafe class name; transport/SQL errors otherwise.
pub async fn class_members(
    client: &IrisClient,
    args: &ClassMembersArgs,
) -> Result<ClassMembersResult> {
    guard(&args.class)?;
    let ns = ns_of(client, args.namespace.as_ref());
    let kind = args.kind.unwrap_or_default();
    let include_inherited = args.include_inherited.unwrap_or(false);
    let origin_filter = if include_inherited {
        String::new()
    } else {
        format!(" AND Origin = '{}'", args.class)
    };
    let want = |k: MembersKind| matches!(kind, MembersKind::All) || kind == k;

    let mut methods = Vec::new();
    let mut properties = Vec::new();
    let mut parameters = Vec::new();
    let mut xdata = Vec::new();

    if want(MembersKind::Methods) {
        let sql = format!(
            "SELECT Name, ClassMethod, SqlProc, FormalSpec, ReturnType, Origin FROM %Dictionary.CompiledMethod WHERE parent = '{}'{} ORDER BY Name",
            args.class, origin_filter
        );
        for row in rows(client, &ns, &sql).await? {
            methods.push(MemberMethod {
                name: str_field(&row, "Name"),
                class_method: truthy(&row, "ClassMethod"),
                sql_proc: truthy(&row, "SqlProc"),
                formal_spec: str_field(&row, "FormalSpec"),
                return_type: str_field(&row, "ReturnType"),
                origin: str_field(&row, "Origin"),
            });
        }
    }
    if want(MembersKind::Properties) {
        let sql = format!(
            "SELECT Name, Type, Collection, Origin FROM %Dictionary.CompiledProperty WHERE parent = '{}'{} ORDER BY Name",
            args.class, origin_filter
        );
        for row in rows(client, &ns, &sql).await? {
            properties.push(MemberProperty {
                name: str_field(&row, "Name"),
                type_: str_field(&row, "Type"),
                collection: str_field(&row, "Collection"),
                origin: str_field(&row, "Origin"),
            });
        }
    }
    if want(MembersKind::Parameters) {
        let sql = format!(
            "SELECT Name, Type, Origin FROM %Dictionary.CompiledParameter WHERE parent = '{}'{} ORDER BY Name",
            args.class, origin_filter
        );
        for row in rows(client, &ns, &sql).await? {
            let t = str_field(&row, "Type");
            parameters.push(MemberParameter {
                type_: if t.is_empty() { None } else { Some(t) },
                name: str_field(&row, "Name"),
                origin: str_field(&row, "Origin"),
            });
        }
    }
    if matches!(kind, MembersKind::All | MembersKind::XData) {
        // XData rows are the class's own declarations: NO Origin filter
        // and no origin field in the entry (E_SPEC §3).
        let sql = format!(
            "SELECT Name FROM %Dictionary.CompiledXData WHERE parent = '{}' ORDER BY Name",
            args.class
        );
        for row in rows(client, &ns, &sql).await? {
            xdata.push(MemberXData {
                name: str_field(&row, "Name"),
            });
        }
    }

    Ok(ClassMembersResult {
        class: args.class.clone(),
        kind: kind_name(kind).to_string(),
        include_inherited,
        counts: MemberCounts {
            methods: methods.len(),
            properties: properties.len(),
            parameters: parameters.len(),
            xdata: xdata.len(),
        },
        methods,
        properties,
        parameters,
        xdata,
    })
}

fn kind_name(k: MembersKind) -> &'static str {
    match k {
        MembersKind::All => "all",
        MembersKind::Methods => "methods",
        MembersKind::Properties => "properties",
        MembersKind::Parameters => "parameters",
        MembersKind::XData => "xdata",
    }
}

/// Direction selector for [`class_dependencies`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDirection {
    /// What this class uses (1-hop dictionary field).
    DependsOn,
    /// Who uses this class (BFS impact walk; `depth` applies here).
    UsedBy,
    /// Both directions.
    #[default]
    Both,
}

/// Arguments for [`class_dependencies`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassDependenciesArgs {
    /// Class to inspect
    pub class: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Which direction to walk (default both)
    pub direction: Option<DependencyDirection>,
    /// Max BFS levels for `used_by` (default 1; >64 clamps)
    pub depth: Option<u32>,
}

/// One dependency edge entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DependencyNode {
    /// Dependency name as written (class, routine, or raw token).
    pub name: String,
    /// `"class"` | `"routine"` | `"other"` (heuristic, `E_SPEC` §3).
    pub kind: String,
    /// BFS level (1 for `depends_on`, which is always 1-hop).
    pub level: u32,
}

/// Result of [`class_dependencies`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClassDependenciesResult {
    /// Inspected class.
    pub class: String,
    /// Effective direction (`"both" | "depends_on" | "used_by"`).
    pub direction: String,
    /// Effective `used_by` depth.
    pub depth: u32,
    /// True when the `used_by` walk hit its frontier/query budget.
    pub truncated: bool,
    /// 1-hop `DependsOn` field entries (empty when direction excludes it).
    pub depends_on: Vec<DependencyNode>,
    /// Impact walk (empty when direction excludes it).
    pub used_by: Vec<DependencyNode>,
}

/// Dependency edges of/for a class via `%Dictionary.ClassDefinition`.
///
/// # Errors
/// [`Error::Config`] on unsafe class name; transport/SQL errors otherwise.
pub async fn class_dependencies(
    client: &IrisClient,
    args: &ClassDependenciesArgs,
) -> Result<ClassDependenciesResult> {
    guard(&args.class)?;
    let ns = ns_of(client, args.namespace.as_ref());
    let direction = args.direction.unwrap_or_default();
    // depends_on is always 1 hop; depth only bounds the used_by walk.
    let depth = match args.depth {
        Some(0) => return Err(Error::Config("depth must be >= 1".to_string())),
        Some(d) => d.min(MAX_DEPTH),
        None => 1,
    };

    let mut depends_on = Vec::new();
    let mut used_by = Vec::new();
    let mut truncated = false;

    if matches!(
        direction,
        DependencyDirection::DependsOn | DependencyDirection::Both
    ) {
        // Deviation (E_BUILD §D3): E_SPEC §3 froze `SELECT DependsOn …`;
        // live probe 2026-10-13 shows IRIS 2025.3 leaves DependsOn EMPTY
        // for inheritance-only classes, while §3's own rule says a Super
        // edge counts as a dependency — so the same row also selects
        // Super and its exact elements are merged (deduped) into the list.
        let sql = format!(
            "SELECT DependsOn, Super FROM %Dictionary.ClassDefinition WHERE Name = '{}'",
            args.class
        );
        let rows = rows(client, &ns, &sql).await?;
        let raw = rows
            .first()
            .and_then(|r| r.get("DependsOn"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        depends_on = classify_depends_on(raw);
        let super_raw = rows
            .first()
            .and_then(|r| r.get("Super"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        for entry in super_raw.split(',').map(str::trim) {
            if entry.is_empty()
                || !valid_class_name(entry)
                || depends_on.iter().any(|d| d.name == entry)
            {
                continue;
            }
            depends_on.push(DependencyNode {
                name: entry.to_string(),
                kind: "class".to_string(),
                level: 1,
            });
        }
    }

    if matches!(
        direction,
        DependencyDirection::UsedBy | DependencyDirection::Both
    ) {
        let (nodes, trunc) = used_by_walk(client, &ns, &args.class, depth).await?;
        used_by = nodes;
        truncated = trunc;
    }

    Ok(ClassDependenciesResult {
        class: args.class.clone(),
        direction: match direction {
            DependencyDirection::DependsOn => "depends_on",
            DependencyDirection::UsedBy => "used_by",
            DependencyDirection::Both => "both",
        }
        .to_string(),
        depth,
        truncated,
        depends_on,
        used_by,
    })
}

/// Split+classify a raw `DependsOn` field (U13): `"Foo.Bar,main^Baz,Qux(1)"`
/// → class/routine/other; `Qux(1)` strips its member suffix first.
fn classify_depends_on(raw: &str) -> Vec<DependencyNode> {
    let mut out = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Strip a trailing "(...)" member suffix if present.
        let token = match entry.find('(') {
            Some(i) => entry[..i].trim(),
            None => entry,
        };
        let name = if token.is_empty() { entry } else { token };
        let kind = if valid_class_name(name) && name.contains('.') {
            "class"
        } else if valid_identifier(name) {
            "routine"
        } else {
            "other"
        };
        out.push(DependencyNode {
            name: name.to_string(),
            kind: kind.to_string(),
            level: 1,
        });
    }
    out
}

/// BFS over `DependsOn [ 'F' OR Super [ 'F'` (impact query, `E_SPEC` §3).
/// A Super edge counts as a dependency. Same exactness + budget rules as
/// the subclass walk.
async fn used_by_walk(
    client: &IrisClient,
    ns: &str,
    start: &str,
    depth: u32,
) -> Result<(Vec<DependencyNode>, bool)> {
    let mut visited: Vec<String> = vec![start.to_string()];
    let mut found: Vec<DependencyNode> = Vec::new();
    let mut frontier: Vec<String> = vec![start.to_string()];
    let mut queries = 0usize;
    let mut truncated = false;
    let mut exhausted = false;

    'levels: for level in 1..=depth {
        let mut next: Vec<String> = Vec::new();
        for f in &frontier {
            if !valid_class_name(f) {
                tracing::warn!("dropping unsafe frontier class name: {f}");
                continue;
            }
            if queries >= FRONTIER_QUERY_BUDGET {
                truncated = true;
                break 'levels;
            }
            queries += 1;
            let sql = format!(
                "SELECT Name, DependsOn, Super FROM %Dictionary.ClassDefinition WHERE DependsOn [ '{f}' OR Super [ '{f}' ORDER BY Name"
            );
            let outcome = sqlapi::query(client, ns, &sql, client.settings().sql_max_rows).await?;
            if outcome.has_more {
                truncated = true;
            }
            for row in &outcome.rows {
                let Some(name) = row.get("Name").and_then(Value::as_str) else {
                    continue;
                };
                let dep_raw = row
                    .get("DependsOn")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let super_raw = row.get("Super").and_then(Value::as_str).unwrap_or_default();
                // Exactness over the substring `[` match (both fields).
                let exact_edge = super_split_exact(super_raw, f) || dep_split_exact(dep_raw, f);
                if !exact_edge || visited.iter().any(|v| v == name) {
                    continue;
                }
                visited.push(name.to_string());
                next.push(name.to_string());
                found.push(DependencyNode {
                    name: name.to_string(),
                    kind: "class".to_string(),
                    level,
                    // (level is already u32)
                });
            }
        }
        if next.is_empty() {
            exhausted = true;
            break;
        }
        frontier = next;
    }
    if !truncated && !exhausted && !frontier.is_empty() {
        // Depth cap reached with a live frontier: deeper users exist.
        truncated = true;
    }
    Ok((found, truncated))
}

/// `Super` is a comma-separated multi-inheritance list (probe: no spaces);
/// the `[` operator is substring-based, so verify an EXACT element edge.
fn super_split_exact(super_raw: &str, f: &str) -> bool {
    super_raw.split(',').map(str::trim).any(|s| s == f)
}

/// Same exactness for `DependsOn` entries (member suffixes stripped first).
fn dep_split_exact(dep_raw: &str, f: &str) -> bool {
    dep_raw.split(',').map(str::trim).any(|s| {
        let s = match s.find('(') {
            Some(i) => s[..i].trim(),
            None => s,
        };
        s == f
    })
}

fn resolve_depth(arg: Option<u32>, default: u32) -> Result<u32> {
    match arg {
        Some(0) => Err(Error::Config("depth must be >= 1".to_string())),
        Some(d) => Ok(d.min(MAX_DEPTH)),
        None => Ok(default.clamp(1, MAX_DEPTH)),
    }
}

async fn rows(client: &IrisClient, ns: &str, sql: &str) -> Result<Vec<Value>> {
    let outcome = sqlapi::query(client, ns, sql, client.settings().sql_max_rows).await?;
    Ok(outcome.rows)
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn truthy(row: &Value, key: &str) -> bool {
    match row.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) != 0 || n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => !s.is_empty() && s != "0",
        _ => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
mod tests {
    use super::*;
    use serde_json::json;

    /// U2 — serde contract for the three arg structs + wire enum values.
    #[test]
    fn graph_args_roundtrip_denies_unknown() {
        let a: FindSubclassesArgs =
            serde_json::from_value(json!({"class": "X", "depth": 3})).unwrap();
        assert_eq!(a.depth, Some(3));
        assert!(
            serde_json::from_value::<FindSubclassesArgs>(json!({"class":"X","nope":1})).is_err()
        );
        assert!(serde_json::from_value::<FindSubclassesArgs>(json!({})).is_err());

        let m: ClassMembersArgs = serde_json::from_value(
            json!({"class": "X", "kind": "xdata", "include_inherited": true}),
        )
        .unwrap();
        assert_eq!(m.kind, Some(MembersKind::XData));
        assert_eq!(
            serde_json::to_value(MembersKind::All).unwrap(),
            json!("all")
        );
        assert_eq!(
            serde_json::to_value(MembersKind::Properties).unwrap(),
            json!("properties")
        );
        assert!(
            serde_json::from_value::<ClassMembersArgs>(json!({"class":"X","kind":"bogus"}))
                .is_err()
        );

        let d: ClassDependenciesArgs =
            serde_json::from_value(json!({"class": "X", "direction": "used_by"})).unwrap();
        assert_eq!(d.direction, Some(DependencyDirection::UsedBy));
        assert_eq!(
            serde_json::to_value(DependencyDirection::DependsOn).unwrap(),
            json!("depends_on")
        );
        assert_eq!(
            serde_json::to_value(DependencyDirection::Both).unwrap(),
            json!("both")
        );
        assert!(
            serde_json::from_value::<ClassDependenciesArgs>(json!({"class":"X","direction":"all"}))
                .is_err()
        );
    }

    /// U13 — `DependsOn` split + classify.
    #[test]
    fn deps_parse_and_classify() {
        let v = classify_depends_on("Foo.Bar,main^Baz,Qux(1)");
        assert_eq!(v.len(), 3);
        // "main^Baz" is not a valid class (caret) nor a plain identifier
        // (^): it lands in "other" unless stripped to a bare routine name.
        assert_eq!(v[0].name, "Foo.Bar");
        assert_eq!(v[0].kind, "class");
        assert_eq!(v[2].name, "Qux");
        assert_eq!(v[2].kind, "routine");
        assert!(classify_depends_on("").is_empty());
        assert!(classify_depends_on(" , ,, ").is_empty());
    }

    #[test]
    fn subclass_edges_are_exact_match() {
        assert!(super_split_exact("A,B", "B"));
        assert!(!super_split_exact(
            "EnsLib.EDI.SEF.NodeList",
            "EnsLib.EDI.SEF.Node"
        ));
        assert!(dep_split_exact("Foo.Bar,Qux(1)", "Qux"));
        assert!(!dep_split_exact("Foo.Bar", "Foo.Ba"));
    }

    #[test]
    fn depth_zero_is_config_error() {
        assert!(matches!(resolve_depth(Some(0), 10), Err(Error::Config(_))));
        assert!(matches!(resolve_depth(Some(100), 10), Ok(64)));
        assert!(matches!(resolve_depth(None, 10), Ok(10)));
    }
}
