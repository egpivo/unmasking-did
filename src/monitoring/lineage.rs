use std::collections::{BTreeSet, HashMap, HashSet};

use crate::storage::ClusterLineageRow;

#[derive(Debug, Clone)]
pub struct ClusterSnapshot {
    pub cluster_id: String,
    pub members: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct LineageConfig {
    pub stable_threshold: f64,
    pub related_threshold: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn should_run_lineage(
    current_chain: &str,
    current_policy_profile_id: &str,
    current_window_start: i64,
    current_window_end: i64,
    previous_chain: &str,
    previous_policy_profile_id: &str,
    previous_window_start: i64,
    previous_window_end: i64,
) -> bool {
    if current_window_start == 0 && current_window_end == 0 {
        return false;
    }
    if previous_window_start == 0 && previous_window_end == 0 {
        return false;
    }
    current_chain == previous_chain && current_policy_profile_id == previous_policy_profile_id
}

#[derive(Debug, Clone)]
struct Candidate {
    current_id: String,
    previous_id: String,
    overlap_count: i64,
    jaccard: f64,
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> (i64, f64) {
    let inter = a.intersection(b).count() as i64;
    if inter == 0 {
        return (0, 0.0);
    }
    let union = a.union(b).count() as f64;
    (inter, inter as f64 / union)
}

pub fn compute_cluster_lineage(
    run_id_current: &str,
    run_id_previous: &str,
    current: &[ClusterSnapshot],
    previous: &[ClusterSnapshot],
    cfg: &LineageConfig,
) -> Vec<ClusterLineageRow> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for cur in current {
        for prev in previous {
            let (overlap, jac) = jaccard(&cur.members, &prev.members);
            if jac >= cfg.related_threshold {
                candidates.push(Candidate {
                    current_id: cur.cluster_id.clone(),
                    previous_id: prev.cluster_id.clone(),
                    overlap_count: overlap,
                    jaccard: jac,
                });
            }
        }
    }

    let mut candidate_current: HashSet<String> = HashSet::new();
    let mut candidate_previous: HashSet<String> = HashSet::new();
    for c in &candidates {
        candidate_current.insert(c.current_id.clone());
        candidate_previous.insert(c.previous_id.clone());
    }

    candidates.sort_by(|a, b| {
        b.jaccard
            .partial_cmp(&a.jaccard)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.overlap_count.cmp(&a.overlap_count))
            .then_with(|| a.current_id.cmp(&b.current_id))
            .then_with(|| a.previous_id.cmp(&b.previous_id))
    });

    let mut used_current: HashSet<String> = HashSet::new();
    let mut used_previous: HashSet<String> = HashSet::new();
    let mut out: Vec<ClusterLineageRow> = Vec::new();

    for c in candidates {
        if used_current.contains(&c.current_id) || used_previous.contains(&c.previous_id) {
            continue;
        }
        used_current.insert(c.current_id.clone());
        used_previous.insert(c.previous_id.clone());
        let label = if c.jaccard >= cfg.stable_threshold {
            "stable"
        } else {
            "related"
        };
        out.push(ClusterLineageRow {
            run_id_current: Some(run_id_current.to_string()),
            cluster_id_current: Some(c.current_id),
            run_id_previous: Some(run_id_previous.to_string()),
            cluster_id_previous: Some(c.previous_id),
            overlap_count: c.overlap_count,
            jaccard: c.jaccard,
            transition_label: label.to_string(),
        });
    }

    // New clusters.
    for cur in current {
        if !candidate_current.contains(&cur.cluster_id) {
            out.push(ClusterLineageRow {
                run_id_current: Some(run_id_current.to_string()),
                cluster_id_current: Some(cur.cluster_id.clone()),
                run_id_previous: Some(run_id_previous.to_string()),
                cluster_id_previous: None,
                overlap_count: 0,
                jaccard: 0.0,
                transition_label: "new".to_string(),
            });
        }
    }

    // Disappeared clusters.
    for prev in previous {
        if !candidate_previous.contains(&prev.cluster_id) {
            out.push(ClusterLineageRow {
                run_id_current: Some(run_id_current.to_string()),
                cluster_id_current: None,
                run_id_previous: Some(run_id_previous.to_string()),
                cluster_id_previous: Some(prev.cluster_id.clone()),
                overlap_count: 0,
                jaccard: 0.0,
                transition_label: "disappeared".to_string(),
            });
        }
    }

    out.sort_by(|a, b| {
        a.transition_label
            .cmp(&b.transition_label)
            .then_with(|| a.cluster_id_current.cmp(&b.cluster_id_current))
            .then_with(|| a.cluster_id_previous.cmp(&b.cluster_id_previous))
    });
    out
}

pub fn cluster_snapshots_from_map(m: &HashMap<String, Vec<String>>) -> Vec<ClusterSnapshot> {
    let mut out: Vec<ClusterSnapshot> = m
        .iter()
        .map(|(id, members)| ClusterSnapshot {
            cluster_id: id.clone(),
            members: members.iter().cloned().collect(),
        })
        .collect();
    out.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));
    out
}
