use std::collections::HashMap;

use unmasking_did::monitoring::lineage::{
    cluster_snapshots_from_map, compute_cluster_lineage, should_run_lineage, LineageConfig,
};

fn snapshots(
    items: &[(&str, &[&str])],
) -> Vec<unmasking_did::monitoring::lineage::ClusterSnapshot> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for (id, members) in items {
        m.insert(
            (*id).to_string(),
            members.iter().map(|x| (*x).to_string()).collect(),
        );
    }
    cluster_snapshots_from_map(&m)
}

#[test]
fn stable_threshold_boundary_is_stable() {
    let prev = snapshots(&[("p1", &["a", "b"])]);
    let cur = snapshots(&[("c1", &["a", "b", "c", "d"])]);
    let rows = compute_cluster_lineage(
        "run_cur",
        "run_prev",
        &cur,
        &prev,
        &LineageConfig {
            stable_threshold: 0.5,
            related_threshold: 0.1,
        },
    );
    assert!(rows.iter().any(|r| {
        r.transition_label == "stable"
            && r.cluster_id_current.as_deref() == Some("c1")
            && r.cluster_id_previous.as_deref() == Some("p1")
    }));
}

#[test]
fn related_threshold_boundary_is_related() {
    let prev = snapshots(&[("p1", &["a", "b", "c", "d", "e"])]);
    let cur = snapshots(&[("c1", &["a", "x", "y", "z", "w"])]);
    let rows = compute_cluster_lineage(
        "run_cur",
        "run_prev",
        &cur,
        &prev,
        &LineageConfig {
            stable_threshold: 0.5,
            related_threshold: 0.1,
        },
    );
    assert!(rows.iter().any(|r| {
        r.transition_label == "related"
            && r.cluster_id_current.as_deref() == Some("c1")
            && r.cluster_id_previous.as_deref() == Some("p1")
    }));
}

#[test]
fn unmatched_clusters_are_new_and_disappeared() {
    let prev = snapshots(&[("p1", &["a", "b"]), ("p2", &["x", "y"])]);
    let cur = snapshots(&[("c1", &["a", "b"]), ("c2", &["m", "n"])]);
    let rows = compute_cluster_lineage(
        "run_cur",
        "run_prev",
        &cur,
        &prev,
        &LineageConfig {
            stable_threshold: 0.5,
            related_threshold: 0.2,
        },
    );
    assert!(rows
        .iter()
        .any(|r| { r.transition_label == "new" && r.cluster_id_current.as_deref() == Some("c2") }));
    assert!(rows.iter().any(|r| {
        r.transition_label == "disappeared" && r.cluster_id_previous.as_deref() == Some("p2")
    }));
}

#[test]
fn output_is_deterministic_under_ties() {
    let prev = snapshots(&[("p1", &["a", "b"]), ("p2", &["a", "b"])]);
    let cur = snapshots(&[("c1", &["a", "b"])]);
    let cfg = LineageConfig {
        stable_threshold: 0.5,
        related_threshold: 0.1,
    };
    let first = compute_cluster_lineage("run_cur", "run_prev", &cur, &prev, &cfg);
    let second = compute_cluster_lineage("run_cur", "run_prev", &cur, &prev, &cfg);
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
}

#[test]
fn current_zero_window_skips_lineage() {
    let ok = should_run_lineage(
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        0,
        0,
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        100,
        200,
    );
    assert!(!ok);
}

#[test]
fn previous_zero_window_skips_lineage() {
    let ok = should_run_lineage(
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        100,
        200,
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        0,
        0,
    );
    assert!(!ok);
}

#[test]
fn explicit_windows_allow_lineage_same_profile_chain() {
    let ok = should_run_lineage(
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        100,
        200,
        "arbitrum",
        "arbitrum_gov_conservative_v1",
        1,
        99,
    );
    assert!(ok);
}

#[test]
fn split_overlap_is_not_marked_new() {
    let prev = snapshots(&[("p1", &["a", "b", "c", "d"])]);
    let cur = snapshots(&[("c1", &["a", "b"]), ("c2", &["c", "d"])]);
    let rows = compute_cluster_lineage(
        "run_cur",
        "run_prev",
        &cur,
        &prev,
        &LineageConfig {
            stable_threshold: 0.5,
            related_threshold: 0.1,
        },
    );
    assert!(
        !rows
            .iter()
            .any(|r| r.transition_label == "new" && r.cluster_id_current.as_deref() == Some("c2")),
        "secondary split successor has predecessor overlap and must not be overclaimed as new"
    );
}

#[test]
fn merge_overlap_is_not_marked_disappeared() {
    let prev = snapshots(&[("p1", &["a", "b"]), ("p2", &["c", "d"])]);
    let cur = snapshots(&[("c1", &["a", "b", "c", "d"])]);
    let rows = compute_cluster_lineage(
        "run_cur",
        "run_prev",
        &cur,
        &prev,
        &LineageConfig {
            stable_threshold: 0.5,
            related_threshold: 0.1,
        },
    );
    assert!(
        !rows.iter().any(|r| {
            r.transition_label == "disappeared"
                && r.cluster_id_previous.as_deref() == Some("p2")
        }),
        "secondary merge predecessor has successor overlap and must not be overclaimed as disappeared"
    );
}
