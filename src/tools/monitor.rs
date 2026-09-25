//! [`monitor_system`] tool: live IRIS metrics → scored snapshot.
//! Port of Prism's `iris/monitor/__init__.rs::collect_snapshot` so snapshots
//! from either tool are comparable (same `KEY_METRICS`, thresholds, grades).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::monitor;
use crate::tools::scoring;

/// Curated metric names surfaced in the snapshot (Prism parity).
const KEY_METRICS: &[&str] = &[
    "iris_cpu_usage",
    "iris_phys_mem_percent_used",
    "iris_page_space_percent_used",
    "iris_smh_total_percent_full",
    "iris_process_count",
    "iris_phys_reads_per_sec",
    "iris_phys_writes_per_sec",
    "iris_glo_ref_per_sec",
    "iris_glo_seize_per_sec",
    "iris_wd_cycle_time",
    "iris_sql_active_queries",
    "iris_cache_efficiency",
    "iris_license_percent_used",
    "iris_license_available",
    "iris_license_consumed",
    "iris_trans_open_count",
    "iris_system_alerts",
    "iris_system_state",
    "iris_db_size_mb",
    "iris_db_free_space",
    "iris_db_max_size_mb",
    "iris_db_latency",
    "iris_db_expansion_size_mb",
    "iris_license_days_remaining",
    "iris_csp_sessions",
    "iris_csp_actual_connections",
    "iris_csp_in_use_connections",
    "iris_csp_gateway_latency",
    "iris_csp_activity",
    "iris_sql_queries_per_second",
    "iris_sql_queries_avg_runtime",
    "iris_trans_open_secs",
    "iris_trans_open_secs_max",
    "iris_smh_total",
    "iris_glo_update_per_sec",
    "iris_ecp_conn",
    "iris_ecp_conn_max",
];

/// One labeled top-process entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProcessEntry {
    /// Process id.
    pub pid: i64,
    /// Commands executed (counter).
    pub commands: i64,
    /// Current routine.
    pub routine: String,
    /// Namespace.
    pub namespace: String,
    /// Job type.
    pub jobtype: String,
}

/// Aggregated values computed from labeled samples.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Aggregated {
    /// Sum of `iris_db_size_mb` in GB.
    pub db_total_size_gb: f64,
    /// Sum of `iris_db_free_space` (MB).
    pub db_total_free_mb: f64,
    /// Sum of `iris_db_max_size_mb` in GB.
    pub db_total_max_gb: f64,
    /// Mean `iris_db_latency` (ms).
    pub db_avg_latency_ms: f64,
    /// Databases reporting a size.
    pub db_count: usize,
    /// CPU per IRIS process type.
    pub cpu_by_type: BTreeMap<String, f64>,
    /// Top 5 processes by commands.
    pub top_processes: Vec<ProcessEntry>,
    /// CSP connections summed across gateways.
    pub csp_total_connections: f64,
    /// CSP in-use connections.
    pub csp_in_use_connections: f64,
    /// Shared memory heap total in GB (metric is KB).
    pub smh_total_gb: f64,
}

/// The full snapshot.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MonitorSnapshot {
    /// Unix seconds when taken.
    pub timestamp: f64,
    /// Composite + per-category load.
    pub score: scoring::LoadScore,
    /// Grade: `idle` | `healthy` | `moderate` | `loaded` | `critical`.
    pub grade: String,
    /// Curated key metrics.
    pub metrics: BTreeMap<String, f64>,
    /// Aggregated values.
    pub aggregated: Aggregated,
    /// Total parsed samples.
    pub metric_count: usize,
    /// Alerts seen (they clear on read).
    pub alerts_count: usize,
}

/// Arguments for [`monitor_system`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorArgs {
    /// Include every raw sample (verbose; default false)
    #[serde(default)]
    pub include_raw_metrics: bool,
}

/// Result of [`monitor_system`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MonitorResult {
    /// The snapshot.
    #[serde(flatten)]
    pub snapshot: MonitorSnapshot,
    /// Raw samples when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_metrics: Option<Vec<monitor::Sample>>,
}

/// Snapshot live IRIS health — behind `rism monitor` and MCP `monitor_system`.
///
/// # Errors
/// Transport errors from the metrics endpoint (alerts failure is tolerated,
/// as in Prism: alerts are best-effort).
pub async fn monitor_system(client: &IrisClient, args: &MonitorArgs) -> Result<MonitorResult> {
    let samples = monitor::metrics(client).await?;

    // Best-effort (Prism parity): alerts failure never blocks the snapshot.
    let alerts_count = match monitor::alerts_raw(client).await {
        // Each alert is one sample; counting avoids any float→int cast.
        Ok(raw) => monitor::parse_prometheus(&raw)
            .iter()
            .filter(|s| s.name == "iris_system_alerts" && s.value > 0.0)
            .count(),
        Err(_) => 0,
    };

    let score = scoring::compute_load_score(&samples);

    let mut metrics = BTreeMap::new();
    for key in KEY_METRICS {
        let hit = samples
            .iter()
            .find(|s| &s.name == key && s.labels.is_empty())
            .or_else(|| samples.iter().find(|s| &s.name == key));
        if let Some(s) = hit {
            metrics.insert((*key).to_string(), finite(s.value));
        }
    }

    let grade = scoring::grade(score.overall).to_string();
    let snapshot = MonitorSnapshot {
        timestamp: current_time(),
        score,
        grade,
        metrics,
        aggregated: build_aggregated(&samples),
        metric_count: samples.len(),
        alerts_count,
    };
    Ok(MonitorResult {
        raw_metrics: args.include_raw_metrics.then_some(samples),
        snapshot,
    })
}

/// Aggregations over labeled samples (Prism's `collect_snapshot` tail).
fn build_aggregated(samples: &[monitor::Sample]) -> Aggregated {
    let finite_sum = |name: &str| -> f64 {
        samples
            .iter()
            .filter(|s| s.name == name)
            .map(|s| s.value)
            .filter(|v| v.is_finite())
            .sum()
    };
    let db_count = samples
        .iter()
        .filter(|s| s.name == "iris_db_size_mb" && s.value.is_finite())
        .count();
    let lat: Vec<f64> = samples
        .iter()
        .filter(|s| s.name == "iris_db_latency" && s.value.is_finite())
        .map(|s| s.value)
        .collect();
    let db_avg_latency_ms = if lat.is_empty() {
        0.0
    } else {
        lat.iter().sum::<f64>() / f64::from(u32::try_from(lat.len()).unwrap_or(u32::MAX))
    };

    let mut cpu_by_type = BTreeMap::new();
    for s in samples {
        if s.name == "iris_cpu_pct" {
            if let Some(id) = s.labels.get("id") {
                cpu_by_type.insert(id.clone(), finite(s.value));
            }
        }
    }

    // top processes: iris_process_commands{id} joined to iris_process labels
    let proc_labels: BTreeMap<String, &monitor::Sample> = samples
        .iter()
        .filter(|s| s.name == "iris_process")
        .filter_map(|s| s.labels.get("id").map(|id| (id.clone(), s)))
        .collect();
    let mut procs: Vec<ProcessEntry> = samples
        .iter()
        .filter(|s| s.name == "iris_process_commands")
        .filter_map(|s| {
            let pid = s.labels.get("id")?.clone();
            let info = proc_labels.get(&pid).map(|x| &x.labels);
            // Monotonic non-negative counters; the rounding cast is exact
            // for every realistic command count (< 2^53).
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let commands = s.value.max(0.0).round() as i64;
            Some(ProcessEntry {
                pid: pid.parse().unwrap_or(0),
                commands,
                routine: label(info, "routine"),
                namespace: label(info, "namespace"),
                jobtype: label(info, "jobtype"),
            })
        })
        .collect();
    procs.sort_by_key(|p| std::cmp::Reverse(p.commands));
    procs.truncate(5);

    let smh_total_gb = samples
        .iter()
        .find(|s| s.name == "iris_smh_total" && s.value.is_finite())
        .map_or(0.0, |s| s.value / 1024.0 / 1024.0);

    Aggregated {
        db_total_size_gb: finite_sum("iris_db_size_mb") / 1024.0,
        db_total_free_mb: finite_sum("iris_db_free_space"),
        db_total_max_gb: finite_sum("iris_db_max_size_mb") / 1024.0,
        db_avg_latency_ms,
        db_count,
        cpu_by_type,
        top_processes: procs,
        csp_total_connections: finite_sum("iris_csp_actual_connections"),
        csp_in_use_connections: finite_sum("iris_csp_in_use_connections"),
        smh_total_gb,
    }
}

fn label(info: Option<&std::collections::BTreeMap<String, String>>, key: &str) -> String {
    info.and_then(|l| l.get(key))
        .cloned()
        .unwrap_or_else(|| "?".to_string())
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn current_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}
