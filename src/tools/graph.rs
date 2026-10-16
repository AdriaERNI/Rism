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

/// F-03: `%Dictionary.Compiled*` keys are the REAL class name, so an alias
/// (`%RegisteredObject`) has no rows at all — returning zero-counts for it
/// reads as "class has no members". 2025.3 has no SQL-visible alias column,
/// so we cannot rewrite alias → canonical server-side; instead a all-zero
/// member read (and an empty `depends_on` row) is cross-checked against
/// `ClassDefinition` so an unknown/alias name answers an honest not-found
/// with a canonical-name hint instead of silent zeros. The existence query's
/// interpolated name is guarded by the same `valid_class_name` call.
async fn class_exists(client: &IrisClient, ns: &str, class: &str) -> Result<bool> {
    let sql = format!("SELECT Name FROM %Dictionary.ClassDefinition WHERE Name = '{class}'");
    Ok(!rows(client, ns, &sql).await?.is_empty())
}

/// Shared honest-not-found text for the F-03 zero-result cross-check.
pub(crate) fn not_found_msg(class: &str, ns: &str) -> Error {
    Error::Config(format!(
        "class '{class}' not found in namespace '{ns}' (no %Dictionary.ClassDefinition row; \
         if this is a class alias such as %RegisteredObject, query the canonical name, \
         e.g. %Library.RegisteredObject)"
    ))
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

/// F-09: same zero-HTTP refusal for the namespace, which interpolates raw
/// into the URL path segment (`api_url`) rather than into SQL.
fn guard_ns(ns: &str) -> Result<()> {
    crate::tools::names::guard_namespace(ns)
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
    guard_ns(&ns)?;
    let depth = resolve_depth(args.depth, client.settings().graph_depth)?;

    let mut visited: Vec<String> = vec![args.class.clone()];
    let mut found: Vec<GraphNode> = Vec::new();
    let mut frontier: Vec<String> = vec![args.class.clone()];
    let mut queries = 0usize;
    let mut truncated = false;
    let mut exhausted = false;

    'levels: for level in 1..=depth {
        let mut next: Vec<String> = Vec::new();
        // One query per frontier CHUNK, not per class: the walk's cost is
        // the round trips, and a single-substring-predicate scan of the
        // class dictionary is already ~15 ms (measured live on 2025.3) —
        // one query per frontier made wide walks quadratic in latency.
        // Names are pre-guarded, so the OR chain can never carry injection.
        for chunk in &super_frontier_chunks(&frontier) {
            if queries >= FRONTIER_QUERY_BUDGET {
                truncated = true;
                break 'levels;
            }
            queries += 1;
            let ors = chunk
                .iter()
                .map(|f| format!("Super [ '{f}'"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT Name, Super FROM %Dictionary.ClassDefinition WHERE {ors} ORDER BY Name"
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
                if !chunk.iter().any(|f| super_split_exact(&super_raw, f)) {
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

    // F-03: an empty walk is either a leaf class or a name the dictionary
    // never keys on (alias/typo) — answer honestly which.
    if found.is_empty() && !class_exists(client, &ns, &args.class).await? {
        return Err(not_found_msg(&args.class, &ns));
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
    /// All structural member tables (methods, properties, parameters,
    /// `XData`, queries, triggers, indices).
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
    /// Queries only (`%Dictionary.CompiledQuery`).
    Queries,
    /// Triggers only (`%Dictionary.CompiledTrigger`).
    Triggers,
    /// Indices only (`%Dictionary.CompiledIndex`).
    Indices,
    /// SQL-callable methods (`CompiledMethod WHERE SqlProc = 1`; there is
    /// no `CompiledProcedure` table on 2025.3 — verified live).
    Procedures,
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

/// One query row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberQuery {
    /// Query name.
    pub name: String,
    /// Formal spec string.
    pub formal_spec: String,
    /// SQL name of the generated stored procedure.
    pub sql_name: String,
    /// Whether an SQL view exists.
    pub sql_view: bool,
    /// Query type (e.g. `%Library.SQLQuery`), fully qualified.
    pub type_: String,
}

/// One trigger row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberTrigger {
    /// Trigger name.
    pub name: String,
    /// Event (`INSERT` / `UPDATE` / `DELETE` / `INSERT/UPDATE`).
    pub event: String,
    /// Timing (`AFTER` / `BEFORE` / `DEFERRED`). The storage column is
    /// `_Time` — `Time` is a reserved word in this dialect and even the
    /// quoted alias is refused (SQLCODE -1; verified live).
    pub time: String,
    /// Code mode (`objectscript` / `sql` / `generator`).
    pub code_mode: String,
}

/// One index row.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberIndex {
    /// Index name.
    pub name: String,
    /// Index type (`key` / `bitmap` / `index` / …).
    pub type_: String,
    /// Indexed properties (comma list as the dictionary stores it).
    pub properties: String,
}

/// One SQL-callable method (a `SqlProc=1` row of `CompiledMethod`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemberProcedure {
    /// Procedure name.
    pub name: String,
    /// Return type (often empty).
    pub return_type: String,
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
    /// Queries returned (0 when the kind did not request them).
    pub queries: usize,
    /// Triggers returned (0 when the kind did not request them).
    pub triggers: usize,
    /// Indices returned (0 when the kind did not request them).
    pub indices: usize,
    /// Procedures returned (only non-zero for `kind = "procedures"`;
    /// `all` reports the `sql_proc` flag on method rows instead).
    pub procedures: usize,
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
    /// Queries (empty when not requested).
    pub queries: Vec<MemberQuery>,
    /// Triggers (empty when not requested).
    pub triggers: Vec<MemberTrigger>,
    /// Indices (empty when not requested).
    pub indices: Vec<MemberIndex>,
    /// SQL-callable methods (empty unless `kind = "procedures"` — `all`
    /// reports them through the `sql_proc` flag on method rows).
    pub procedures: Vec<MemberProcedure>,
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
    guard_ns(&ns)?;
    let origin_filter = if include_inherited {
        String::new()
    } else {
        format!(" AND Origin = '{}'", args.class)
    };
    let want = |k: MembersKind| matches!(kind, MembersKind::All) || kind == k;

    let methods = if want(MembersKind::Methods) {
        methods_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let properties = if want(MembersKind::Properties) {
        properties_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let parameters = if want(MembersKind::Parameters) {
        parameters_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let xdata = if matches!(kind, MembersKind::All | MembersKind::XData) {
        xdata_of(client, &ns, &args.class).await?
    } else {
        Vec::new()
    };
    let queries = if want(MembersKind::Queries) {
        queries_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let triggers = if want(MembersKind::Triggers) {
        triggers_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let indices = if want(MembersKind::Indices) {
        indices_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };
    let procedures = if matches!(kind, MembersKind::Procedures) {
        procedures_of(client, &ns, &args.class, &origin_filter).await?
    } else {
        Vec::new()
    };

    // F-03: an all-zero read means either a genuinely empty class or a name
    // the Compiled* tables never key on (alias like %RegisteredObject, or a
    // typo). Distinguish them via ClassDefinition: no row -> honest
    // not-found instead of misleading zeros.
    if methods.is_empty()
        && properties.is_empty()
        && parameters.is_empty()
        && xdata.is_empty()
        && queries.is_empty()
        && triggers.is_empty()
        && indices.is_empty()
        && procedures.is_empty()
        && !class_exists(client, &ns, &args.class).await?
    {
        return Err(not_found_msg(&args.class, &ns));
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
            queries: queries.len(),
            triggers: triggers.len(),
            indices: indices.len(),
            procedures: procedures.len(),
        },
        methods,
        properties,
        parameters,
        xdata,
        queries,
        triggers,
        indices,
        procedures,
    })
}

fn kind_name(k: MembersKind) -> &'static str {
    match k {
        MembersKind::All => "all",
        MembersKind::Methods => "methods",
        MembersKind::Properties => "properties",
        MembersKind::Parameters => "parameters",
        MembersKind::XData => "xdata",
        MembersKind::Queries => "queries",
        MembersKind::Triggers => "triggers",
        MembersKind::Indices => "indices",
        MembersKind::Procedures => "procedures",
    }
}

/// `%Dictionary.CompiledMethod` rows for one class.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn methods_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberMethod>> {
    let sql = format!(
        "SELECT Name, ClassMethod, SqlProc, FormalSpec, ReturnType, Origin FROM %Dictionary.CompiledMethod WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberMethod {
            name: str_field(&row, "Name"),
            class_method: truthy(&row, "ClassMethod"),
            sql_proc: truthy(&row, "SqlProc"),
            formal_spec: str_field(&row, "FormalSpec"),
            return_type: str_field(&row, "ReturnType"),
            origin: str_field(&row, "Origin"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledProperty` rows for one class.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn properties_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberProperty>> {
    let sql = format!(
        "SELECT Name, Type, Collection, Origin FROM %Dictionary.CompiledProperty WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberProperty {
            name: str_field(&row, "Name"),
            type_: str_field(&row, "Type"),
            collection: str_field(&row, "Collection"),
            origin: str_field(&row, "Origin"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledParameter` rows for one class.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn parameters_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberParameter>> {
    let sql = format!(
        "SELECT Name, Type, Origin FROM %Dictionary.CompiledParameter WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        let t = str_field(&row, "Type");
        out.push(MemberParameter {
            type_: if t.is_empty() { None } else { Some(t) },
            name: str_field(&row, "Name"),
            origin: str_field(&row, "Origin"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledXData` names for one class. `XData` rows are the
/// class's own declarations: NO Origin filter and no origin field in the
/// entry (`E_SPEC` §3).
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn xdata_of(client: &IrisClient, ns: &str, class: &str) -> Result<Vec<MemberXData>> {
    let sql = format!(
        "SELECT Name FROM %Dictionary.CompiledXData WHERE parent = '{class}' ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberXData {
            name: str_field(&row, "Name"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledQuery` rows for one class.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn queries_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberQuery>> {
    let sql = format!(
        "SELECT Name, FormalSpec, SqlName, SqlView, Type FROM %Dictionary.CompiledQuery WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberQuery {
            name: str_field(&row, "Name"),
            formal_spec: str_field(&row, "FormalSpec"),
            sql_name: str_field(&row, "SqlName"),
            sql_view: truthy(&row, "SqlView"),
            type_: str_field(&row, "Type"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledTrigger` rows for one class. The storage column is
/// `_Time` field — bare `Time` is a reserved word (SQLCODE -1) and even a quoted
/// alias fails on 2025.3 (verified live), so the result field simply
/// carries the `_Time` value.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn triggers_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberTrigger>> {
    let sql = format!(
        "SELECT Name, Event, _Time, CodeMode FROM %Dictionary.CompiledTrigger WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberTrigger {
            name: str_field(&row, "Name"),
            event: str_field(&row, "Event"),
            time: str_field(&row, "_Time"),
            code_mode: str_field(&row, "CodeMode"),
        });
    }
    Ok(out)
}

/// `%Dictionary.CompiledIndex` rows for one class.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn indices_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberIndex>> {
    let sql = format!(
        "SELECT Name, Type, Properties FROM %Dictionary.CompiledIndex WHERE parent = '{class}'{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberIndex {
            name: str_field(&row, "Name"),
            type_: str_field(&row, "Type"),
            properties: str_field(&row, "Properties"),
        });
    }
    Ok(out)
}

/// SQL-callable methods: no `%Dictionary.CompiledProcedure` table exists
/// (SQLCODE -30, verified live) — procedures ARE SqlProc=1 compiled
/// methods.
///
/// # Errors
/// SQL/transport errors from the dictionary query.
async fn procedures_of(
    client: &IrisClient,
    ns: &str,
    class: &str,
    origin_filter: &str,
) -> Result<Vec<MemberProcedure>> {
    let sql = format!(
        "SELECT Name, ReturnType FROM %Dictionary.CompiledMethod WHERE parent = '{class}' AND SqlProc = 1{origin_filter} ORDER BY Name"
    );
    let mut out = Vec::new();
    for row in rows(client, ns, &sql).await? {
        out.push(MemberProcedure {
            name: str_field(&row, "Name"),
            return_type: str_field(&row, "ReturnType"),
        });
    }
    Ok(out)
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
    guard_ns(&ns)?;
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

    // F-03: both lists empty on a name with no dictionary row -> the class
    // (alias/typo) does not exist; say so instead of returning empty edges.
    if depends_on.is_empty()
        && used_by.is_empty()
        && !class_exists(client, &ns, &args.class).await?
    {
        return Err(not_found_msg(&args.class, &ns));
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
        for chunk in &super_frontier_chunks(&frontier) {
            if queries >= FRONTIER_QUERY_BUDGET {
                truncated = true;
                break 'levels;
            }
            queries += 1;
            let ors = chunk
                .iter()
                .flat_map(|f| [format!("DependsOn [ '{f}'"), format!("Super [ '{f}'")])
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT Name, DependsOn, Super FROM %Dictionary.ClassDefinition WHERE {ors} ORDER BY Name"
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
                let exact_edge = chunk
                    .iter()
                    .any(|f| super_split_exact(super_raw, f) || dep_split_exact(dep_raw, f));
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

/// Chunk a BFS frontier for one-query-per-chunk walks: guarded names,
/// deduped, capped per statement so the OR chain stays well under the
/// server's SQL length limits.
fn super_frontier_chunks(frontier: &[String]) -> Vec<Vec<String>> {
    let mut seen: Vec<String> = Vec::new();
    for f in frontier {
        // Server-sourced frontier names are NOT trusted: re-guard.
        if !valid_class_name(f) {
            tracing::warn!("dropping unsafe frontier class name: {f}");
            continue;
        }
        if !seen.iter().any(|v| v == f) {
            seen.push(f.clone());
        }
    }
    // The dictionary is a few thousand rows; each extra substring predicate
    // adds a marginal scan cost, so keep chunks small enough that a wide
    // level still fans out only linearly in total scan work.
    seen.chunks(48).map(<[String]>::to_vec).collect()
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
        // Census kinds (additive wire strings).
        for (k, w) in [
            (MembersKind::Queries, "queries"),
            (MembersKind::Triggers, "triggers"),
            (MembersKind::Indices, "indices"),
            (MembersKind::Procedures, "procedures"),
        ] {
            assert_eq!(serde_json::to_value(k).unwrap(), json!(w));
            let back: MembersKind = serde_json::from_value(json!(w)).unwrap();
            assert_eq!(back, k);
        }
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

    /// Frontier batching: unsafe names drop (never reach the SQL), dupes
    /// collapse, and a chunk never exceeds the statement budget.
    #[test]
    fn frontier_chunks_guard_dedupe_and_cap() {
        let f: Vec<String> = ["A.B", "A.B", "C.D;DROP", "%Library.String"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let chunks = super_frontier_chunks(&f);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            vec!["A.B".to_string(), "%Library.String".to_string()]
        );
        let wide: Vec<String> = (0..200).map(|i| format!("P.C{i}")).collect();
        let chunks = super_frontier_chunks(&wide);
        assert!(chunks.iter().all(|c| c.len() <= 48));
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 200);
    }
}
