use chrono::{Duration, Utc};
use std::collections::HashMap;
use RaceboardServer::{
    cluster::RaceCluster,
    hnsw_dbscan::{HnswDBSCAN, ValidationCriteria, ValidationMetrics, ValidationResult},
    models::{Race, RaceState},
    phased_rollout::{PhasedRollout, RolloutConfig, RolloutPhase},
    rebuild::{
        adjusted_rand_index, assign_noise_as_border, calculate_average_cohesion,
        calculate_noise_ratio, custom_distance, detect_knee_kneedle, map_stable_ids,
        race_to_vector, silhouette_sampled, ClusterId, CorePointIndex, Curve, DBSCANResult,
        Direction, MappingThresholds, RaceId, SourceConfig,
    },
    stats::ExecutionStats,
};

fn create_test_race(id: &str, source: &str, title: &str, duration_sec: i64) -> Race {
    let mut metadata = HashMap::new();
    metadata.insert("duration".to_string(), duration_sec.to_string());
    metadata.insert("project".to_string(), "test-project".to_string());

    Race {
        id: id.to_string(),
        source: source.to_string(),
        title: title.to_string(),
        state: RaceState::Passed,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        duration_sec: Some(duration_sec),
        eta_sec: None,
        progress: Some(100),
        deeplink: None,
        metadata: Some(metadata),
        events: Some(vec![]),
    }
}

fn create_test_races() -> Vec<Race> {
    vec![
        // Cluster 1: cargo build commands (should cluster together)
        create_test_race("r1", "cargo", "cargo build", 30),
        create_test_race("r2", "cargo", "cargo build --release", 45),
        create_test_race("r3", "cargo", "cargo build --features foo", 35),
        create_test_race("r4", "cargo", "cargo build", 32),
        create_test_race("r5", "cargo", "cargo build --release", 48),
        // Cluster 2: cargo test commands (should cluster together)
        create_test_race("r6", "cargo", "cargo test", 120),
        create_test_race("r7", "cargo", "cargo test --all", 150),
        create_test_race("r8", "cargo", "cargo test integration", 130),
        create_test_race("r9", "cargo", "cargo test unit", 110),
        // Cluster 3: npm commands (different source)
        create_test_race("r10", "npm", "npm install", 60),
        create_test_race("r11", "npm", "npm install --save-dev", 65),
        create_test_race("r12", "npm", "npm build", 40),
        create_test_race("r13", "npm", "npm run build", 42),
        // Noise points (unique commands)
        create_test_race("r14", "cargo", "cargo doc --open", 15),
        create_test_race("r15", "npm", "npm audit fix", 25),
        create_test_race("r16", "cargo", "cargo clippy -- -W warnings", 20),
    ]
}

#[tokio::test]
async fn test_hnsw_dbscan_clustering() {
    println!("\n=== Testing HNSW-Optimized DBSCAN Clustering ===\n");

    let config = SourceConfig {
        eps_range: (0.3, 0.5),
        min_samples: 2,
        min_cluster_size: 2,
        preserve_bootstraps: true,
        w_title: 0.6,
        w_meta: 0.4,
        tau_match: 0.5,
        tau_split: 0.35,
        tau_merge_lo: 0.35,
        tau_merge_hi: 0.6,
        last_eps: None,
    };

    let races = create_test_races();
    let cargo_races: Vec<Race> = races
        .iter()
        .filter(|r| r.source == "cargo")
        .cloned()
        .collect();

    // Test HNSW-based clustering
    let mut hnsw_dbscan = HnswDBSCAN::new(config.clone(), 1000);

    // Build index
    let result = hnsw_dbscan.build_index(cargo_races.clone());
    assert!(result.is_ok(), "Failed to build HNSW index: {:?}", result);

    // Run DBSCAN with eps=0.4
    let dbscan_result = hnsw_dbscan.run_dbscan(0.4, 2);

    // Verify clustering results
    assert!(
        dbscan_result.clusters.len() >= 2,
        "Expected at least 2 clusters, got {}",
        dbscan_result.clusters.len()
    );
    assert!(
        dbscan_result.noise.len() <= 3,
        "Too many noise points: {}",
        dbscan_result.noise.len()
    );

    println!("HNSW DBSCAN Results:");
    println!("  Clusters found: {}", dbscan_result.clusters.len());
    println!("  Noise points: {}", dbscan_result.noise.len());
    println!("  Border points: {}", dbscan_result.border_points.len());

    for (cluster_id, members) in &dbscan_result.clusters {
        println!("  Cluster {}: {} members", cluster_id, members.len());
    }
}

#[tokio::test]
async fn test_validation_metrics() {
    println!("\n=== Testing Validation Metrics ===\n");

    let config = SourceConfig {
        eps_range: (0.3, 0.5),
        min_samples: 2,
        min_cluster_size: 2,
        preserve_bootstraps: false,
        w_title: 0.6,
        w_meta: 0.4,
        tau_match: 0.5,
        tau_split: 0.35,
        tau_merge_lo: 0.35,
        tau_merge_hi: 0.6,
        last_eps: None,
    };

    let races = create_test_races();

    // Create clusters for testing
    let mut clusters: HashMap<ClusterId, RaceCluster> = HashMap::new();

    // Cluster 1: cargo build commands
    let mut stats1 = ExecutionStats::new();
    stats1.update_with_duration(30);
    stats1.update_with_duration(45);
    stats1.update_with_duration(35);

    clusters.insert(
        "cargo:cluster_0".to_string(),
        RaceCluster {
            cluster_id: "cargo:cluster_0".to_string(),
            source: "cargo".to_string(),
            representative_title: "cargo build".to_string(),
            representative_metadata: HashMap::new(),
            stats: stats1,
            member_race_ids: vec!["r1".to_string(), "r2".to_string(), "r3".to_string()],
            member_titles: vec![
                "cargo build".to_string(),
                "cargo build --release".to_string(),
                "cargo build --features foo".to_string(),
            ],
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        },
    );

    // Cluster 2: cargo test commands
    let mut stats2 = ExecutionStats::new();
    stats2.update_with_duration(120);
    stats2.update_with_duration(150);
    stats2.update_with_duration(130);

    clusters.insert(
        "cargo:cluster_1".to_string(),
        RaceCluster {
            cluster_id: "cargo:cluster_1".to_string(),
            source: "cargo".to_string(),
            representative_title: "cargo test".to_string(),
            representative_metadata: HashMap::new(),
            stats: stats2,
            member_race_ids: vec!["r6".to_string(), "r7".to_string(), "r8".to_string()],
            member_titles: vec![
                "cargo test".to_string(),
                "cargo test --all".to_string(),
                "cargo test integration".to_string(),
            ],
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        },
    );

    // Test noise ratio
    let noise_ratio = calculate_noise_ratio(&clusters);
    println!("Noise Ratio: {:.2}%", noise_ratio * 100.0);
    assert!(noise_ratio < 0.5, "Noise ratio too high");

    // Test average cohesion
    let cohesion = calculate_average_cohesion(&clusters);
    println!("Average Cohesion: {:.3}", cohesion);
    assert!(cohesion > 0.5, "Cohesion too low");

    // Test silhouette coefficient
    let silhouette = silhouette_sampled(&clusters, &races, &config, 10);
    println!("Silhouette Coefficient: {:.3}", silhouette);

    // Test ARI between two clusterings
    let ari = adjusted_rand_index(&clusters, &clusters);
    println!("Adjusted Rand Index (self): {:.3}", ari);
    assert!(
        ari > 0.9,
        "ARI should be close to 1.0 for identical clusterings"
    );
}

#[tokio::test]
async fn test_knee_detection() {
    println!("\n=== Testing Knee Detection (Kneedle Algorithm) ===\n");

    // Create k-distance graph data (sorted distances)
    let k_distances = vec![
        0.1, 0.15, 0.2, 0.22, 0.25, 0.28, 0.3, 0.35, // gradual increase
        0.4, 0.5, 0.65, 0.8, 0.95, 1.1, 1.3, 1.5, // sharp increase (knee around 0.4)
    ];

    let knee = detect_knee_kneedle(&k_distances, Curve::Convex, Direction::Increasing, 1.0);

    assert!(knee.is_some(), "Failed to detect knee");
    let knee_value = knee.unwrap();
    println!("Detected knee at: {:.2}", knee_value);
    assert!(
        knee_value > 0.2 && knee_value < 0.6,
        "Knee detected outside expected range"
    );
}

#[tokio::test]
async fn test_stable_id_mapping() {
    println!("\n=== Testing Stable ID Mapping ===\n");

    // Create old clusters
    let mut old_clusters: HashMap<ClusterId, RaceCluster> = HashMap::new();
    old_clusters.insert(
        "cargo:cluster_0".to_string(),
        RaceCluster {
            cluster_id: "cargo:cluster_0".to_string(),
            source: "cargo".to_string(),
            representative_title: "cargo build".to_string(),
            representative_metadata: HashMap::new(),
            stats: ExecutionStats::default(),
            member_race_ids: vec!["r1".to_string(), "r2".to_string(), "r3".to_string()],
            member_titles: vec![
                "cargo build".to_string(),
                "cargo build --release".to_string(),
                "cargo build --features foo".to_string(),
            ],
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        },
    );

    // Create new clusters (slightly different but overlapping)
    let mut new_clusters: HashMap<ClusterId, RaceCluster> = HashMap::new();
    new_clusters.insert(
        "cargo:cluster_temp_0".to_string(),
        RaceCluster {
            cluster_id: "cargo:cluster_temp_0".to_string(),
            source: "cargo".to_string(),
            representative_title: "cargo build".to_string(),
            representative_metadata: HashMap::new(),
            stats: ExecutionStats::default(),
            member_race_ids: vec!["r1".to_string(), "r2".to_string(), "r4".to_string()],
            member_titles: vec![
                "cargo build".to_string(),
                "cargo build --release".to_string(),
                "cargo build".to_string(),
            ],
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        },
    );

    let thresholds = MappingThresholds {
        tau_match: 0.5,
        tau_split: 0.35,
        tau_merge_lo: 0.35,
        tau_merge_hi: 0.6,
    };

    let mapping = map_stable_ids(&old_clusters, &new_clusters, thresholds);

    println!("ID Mapping:");
    for (new_id, old_id) in &mapping {
        println!("  {} -> {}", new_id, old_id);
    }

    assert_eq!(mapping.len(), 1, "Expected one mapping");
    assert_eq!(
        mapping.get("cargo:cluster_temp_0"),
        Some(&"cargo:cluster_0".to_string()),
        "Incorrect ID mapping"
    );
}

#[tokio::test]
async fn test_phased_rollout() {
    println!("\n=== Testing Phased Rollout System ===\n");

    let config = RolloutConfig {
        pilot_source: "cargo".to_string(),
        shadow_duration: Duration::hours(1),
        canary_duration: Duration::hours(2),
        canary_percentage: 10,
        success_threshold: 0.9,
        min_rebuilds_for_promotion: 3,
        auto_rollback: true,
        validation_criteria: ValidationCriteria::default(),
    };

    let mut rollout = PhasedRollout::new(config);
    rollout.register_source("cargo");

    // Start Phase 1
    let result = rollout.start_phase_1();
    assert!(result.is_ok(), "Failed to start Phase 1");
    assert_eq!(rollout.current_phase, RolloutPhase::SingleSource);

    // Simulate successful rebuilds
    for _ in 0..5 {
        let validation_result = ValidationResult {
            passed: true,
            metrics: ValidationMetrics::default(),
            mae_increase: 0.02,
            failures: vec![],
        };
        rollout.record_rebuild_result("cargo", validation_result);
    }

    // Promote to canary
    let result = rollout.promote_to_canary("cargo");
    assert!(result.is_ok(), "Failed to promote to canary");

    // Promote to production
    let result = rollout.promote_to_production("cargo");
    assert!(result.is_ok(), "Failed to promote to production");

    // Try to advance phase
    let advanced = rollout.try_advance_phase().unwrap();
    assert!(advanced, "Failed to advance phase");
    assert_eq!(rollout.current_phase, RolloutPhase::AllSourcesConservative);

    println!("Rollout Status:");
    println!("  Current Phase: {:?}", rollout.current_phase);
    println!(
        "  Total Rebuilds: {}",
        rollout.global_metrics.total_rebuilds
    );
    println!(
        "  Success Rate: {:.2}%",
        rollout.global_metrics.successful_rebuilds as f64
            / rollout.global_metrics.total_rebuilds as f64
            * 100.0
    );

    // Test rollback on failures
    for _ in 0..3 {
        let validation_result = ValidationResult {
            passed: false,
            metrics: ValidationMetrics::default(),
            mae_increase: 0.3,
            failures: vec!["High MAE increase".to_string()],
        };
        rollout.record_rebuild_result("npm", validation_result);
    }

    // Check if rollback was triggered
    if rollout.current_phase == RolloutPhase::Rollback {
        println!("  Rollback triggered due to failures");
    }
}

#[tokio::test]
async fn test_full_rebuild_pipeline() {
    println!("\n=== Testing Full Rebuild Pipeline ===\n");

    let config = SourceConfig {
        eps_range: (0.3, 0.5),
        min_samples: 2,
        min_cluster_size: 2,
        preserve_bootstraps: true,
        w_title: 0.6,
        w_meta: 0.4,
        tau_match: 0.5,
        tau_split: 0.35,
        tau_merge_lo: 0.35,
        tau_merge_hi: 0.6,
        last_eps: None,
    };

    let races = create_test_races();

    // Test with HNSW-based clustering
    let mut hnsw_dbscan = HnswDBSCAN::new(config.clone(), 1000);

    // Build index for all races
    let result = hnsw_dbscan.build_index(races.clone());
    assert!(result.is_ok(), "Failed to build HNSW index");

    // Run clustering with automatic parameter selection
    let eps_values = vec![0.3, 0.35, 0.4, 0.45, 0.5];
    let mut best_result = None;
    let mut best_score = f64::NEG_INFINITY;

    for eps in eps_values {
        let dbscan_result = hnsw_dbscan.run_dbscan(eps, config.min_samples);

        // Simple scoring based on cluster count and noise ratio
        let num_clusters = dbscan_result.clusters.len();
        let noise_ratio = dbscan_result.noise.len() as f64 / races.len() as f64;

        // Prefer more clusters with less noise
        let score = num_clusters as f64 - noise_ratio * 10.0;

        if score > best_score {
            best_score = score;
            best_result = Some((eps, dbscan_result));
        }
    }

    if let Some((best_eps, result)) = best_result {
        println!("Best parameters found:");
        println!("  eps: {:.2}", best_eps);
        println!("  Clusters: {}", result.clusters.len());
        println!("  Noise points: {}", result.noise.len());
        println!("  Border points: {}", result.border_points.len());

        // Test noise assignment
        if !result.noise.is_empty() {
            let core_index = hnsw_dbscan.build_core_index(&result);
            let noise_races: Vec<Race> = races
                .iter()
                .filter(|r| result.noise.contains(&r.id))
                .cloned()
                .collect();

            let border_assignments =
                assign_noise_as_border(&noise_races, &core_index, best_eps, &config);

            println!("\n  Noise assignment results:");
            println!(
                "  {} of {} noise points assigned as border points",
                border_assignments.len(),
                result.noise.len()
            );
        }
    }
}

#[tokio::test]
async fn test_vector_representation() {
    println!("\n=== Testing Vector Representation ===\n");

    let race = create_test_race("test", "cargo", "cargo build --release", 45);
    let vector = race_to_vector(&race);

    println!(
        "Vector representation for '{}' has {} dimensions",
        race.title,
        vector.len()
    );
    assert!(!vector.is_empty(), "Vector should not be empty");

    // Test that similar races produce similar vectors
    let race2 = create_test_race("test2", "cargo", "cargo build", 30);
    let vector2 = race_to_vector(&race2);

    let distance: f32 = vector
        .iter()
        .zip(vector2.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt();

    println!("Euclidean distance between similar races: {:.3}", distance);
}

#[tokio::test]
async fn test_custom_distance_metric() {
    println!("\n=== Testing Custom Distance Metric ===\n");

    let config = SourceConfig {
        eps_range: (0.3, 0.5),
        min_samples: 2,
        min_cluster_size: 2,
        preserve_bootstraps: false,
        w_title: 0.6,
        w_meta: 0.4,
        tau_match: 0.5,
        tau_split: 0.35,
        tau_merge_lo: 0.35,
        tau_merge_hi: 0.6,
        last_eps: None,
    };

    // Test similar races
    let race1 = create_test_race("r1", "cargo", "cargo build", 30);
    let race2 = create_test_race("r2", "cargo", "cargo build --release", 45);
    let distance = custom_distance(&race1, &race2, &config);
    println!(
        "Distance between 'cargo build' and 'cargo build --release': {:.3}",
        distance
    );
    assert!(distance < 0.5, "Similar commands should have low distance");

    // Test different races
    let race3 = create_test_race("r3", "cargo", "cargo test", 120);
    let distance2 = custom_distance(&race1, &race3, &config);
    println!(
        "Distance between 'cargo build' and 'cargo test': {:.3}",
        distance2
    );
    assert!(
        distance2 > distance,
        "Different commands should have higher distance"
    );

    // Test races from different sources
    let race4 = create_test_race("r4", "npm", "npm build", 40);
    let distance3 = custom_distance(&race1, &race4, &config);
    println!(
        "Distance between 'cargo build' and 'npm build': {:.3}",
        distance3
    );
    assert!(
        distance3 == 1.0,
        "Different sources should have maximum distance"
    );
}
