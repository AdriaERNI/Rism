//! Load scoring over parsed metrics — thresholds and aggregation rules are
//! a 1:1 port of Prism's `monitor/scorer.py` so cross-tool comparisons hold.

use schemars::JsonSchema;
use serde::Serialize;

use crate::iris::monitor::Sample;

fn cpu_thresholds() -> [(&'static str, f64); 1] {
    [("iris_cpu_usage", 100.0)]
}

fn memory_thresholds() -> [(&'static str, f64); 3] {
    [
        ("iris_phys_mem_percent_used", 100.0),
        ("iris_page_space_percent_used", 100.0),
        ("iris_smh_total_percent_full", 100.0),
    ]
}

fn disk_thresholds() -> [(&'static str, f64); 4] {
    [
        ("iris_phys_reads_per_sec", 5000.0),
        ("iris_phys_writes_per_sec", 3000.0),
        ("iris_db_latency", 50.0),
        ("iris_disk_percent_full", 100.0),
    ]
}

fn process_thresholds() -> [(&'static str, f64); 5] {
    [
        ("iris_process_count", 500.0),
        ("iris_glo_seize_per_sec", 50.0),
        ("iris_wd_cycle_time", 2000.0),
        ("iris_sql_active_queries", 100.0),
        ("iris_trans_open_count", 50.0),
    ]
}

/// Value → 0..=100; NaN/±Inf count as missing (0), never penalised.
fn normalise(value: f64, threshold: f64) -> f64 {
    if value.is_nan() || value.is_infinite() {
        return 0.0;
    }
    if threshold <= 0.0 {
        return if value > 0.0 { 100.0 } else { 0.0 };
    }
    ((value / threshold) * 100.0).clamp(0.0, 100.0)
}

fn score_category(samples: &[Sample], thresholds: &[(&str, f64)], special: Option<&str>) -> f64 {
    let mut components = Vec::new();
    if let Some(name) = special {
        let values: Vec<f64> = samples
            .iter()
            .filter(|s| s.name == name)
            .map(|s| s.value)
            .collect();
        if !values.is_empty() {
            let total: f64 = values
                .iter()
                .filter(|v| !v.is_nan() && !v.is_infinite())
                .sum();
            components.push(normalise(total, 100.0));
        }
    }
    for (name, threshold) in thresholds {
        let valid: Vec<f64> = samples
            .iter()
            .filter(|s| &s.name == name)
            .map(|s| s.value)
            .filter(|v| !v.is_nan() && !v.is_infinite())
            .collect();
        if !valid.is_empty() {
            let max_value = valid.iter().copied().fold(f64::MIN, f64::max);
            components.push(normalise(max_value, *threshold));
        }
    }
    if components.is_empty() {
        return 0.0;
    }
    (components.iter().sum::<f64>()
        / f64::from(u32::try_from(components.len()).unwrap_or(u32::MAX)))
    .min(100.0)
}

/// Composite + per-category 0..=100 load (higher = busier).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LoadScore {
    /// Weighted overall (25% each category).
    pub overall: f64,
    /// CPU sub-score.
    pub cpu: f64,
    /// Memory sub-score.
    pub memory: f64,
    /// Disk/IO sub-score.
    pub disk: f64,
    /// Process sub-score.
    pub process: f64,
}

/// Compute the load score (Prism-parity rules).
#[must_use]
pub fn compute_load_score(samples: &[Sample]) -> LoadScore {
    let cpu = score_category(samples, &cpu_thresholds(), Some("iris_cpu_pct"));
    let memory = score_category(samples, &memory_thresholds(), None);
    let disk = score_category(samples, &disk_thresholds(), None);
    let process = score_category(samples, &process_thresholds(), None);
    let overall = (cpu + memory + disk + process) / 4.0;
    let r = |v: f64| (v * 100.0).round() / 100.0;
    LoadScore {
        overall: r(overall),
        cpu: r(cpu),
        memory: r(memory),
        disk: r(disk),
        process: r(process),
    }
}

/// idle <10 ≤ healthy <30 ≤ moderate <60 ≤ loaded <80 ≤ critical.
#[must_use]
pub fn grade(score: f64) -> &'static str {
    let s = score.clamp(0.0, 100.0);
    if s < 10.0 {
        "idle"
    } else if s < 30.0 {
        "healthy"
    } else if s < 60.0 {
        "moderate"
    } else if s < 80.0 {
        "loaded"
    } else {
        "critical"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use crate::iris::monitor::parse_prometheus;
    use std::collections::BTreeMap;

    fn sample(name: &str, value: f64) -> Sample {
        Sample {
            name: name.into(),
            labels: BTreeMap::new(),
            value,
        }
    }

    #[test]
    fn idle_server_scores_low() {
        let s = vec![
            sample("iris_cpu_usage", 0.0),
            sample("iris_process_count", 12.0),
        ];
        let score = compute_load_score(&s);
        assert!(score.overall < 10.0, "{score:?}");
        assert_eq!(grade(score.overall), "idle");
    }

    #[test]
    fn saturation_maps_to_critical() {
        let s = vec![
            sample("iris_cpu_usage", 100.0),
            sample("iris_phys_mem_percent_used", 95.0),
            sample("iris_phys_reads_per_sec", 6000.0),
            sample("iris_process_count", 500.0),
        ];
        assert_eq!(grade(compute_load_score(&s).overall), "critical");
    }

    #[test]
    fn nan_values_are_missing_not_penalised() {
        let s = vec![sample("iris_cpu_usage", f64::NAN)];
        assert!(compute_load_score(&s).overall.abs() < f64::EPSILON);
    }

    #[test]
    fn cpu_pct_labels_sum_across_process_types() {
        // special metric: SUM across labeled samples vs 100
        let text = "iris_cpu_pct{id=\"AUXWD\"} 20\niris_cpu_pct{id=\"CSPSRV\"} 30\n";
        let samples = parse_prometheus(text);
        let score = compute_load_score(&samples);
        assert!((score.cpu - 50.0).abs() < 0.01, "{score:?}");
    }

    #[test]
    fn multiple_samples_take_max() {
        let text = "iris_db_latency{id=\"MGR\"} 4\niris_db_latency{id=\"USER\"} 12\n";
        let samples = parse_prometheus(text);
        // disk: latency 12/50*100 = 24 only component
        let score = compute_load_score(&samples);
        assert!((score.disk - 24.0).abs() < 0.01, "{score:?}");
    }
}
