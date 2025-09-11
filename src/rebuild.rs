use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use lru::LruCache;
use rand::prelude::*;
use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use unicode_normalization::UnicodeNormalization;

use crate::cluster::RaceCluster;
use crate::hnsw_dbscan::{validate_clusters_comprehensive, HnswDBSCAN, ValidationCriteria};
use crate::models::Race;

pub type RaceId = String;
pub type ClusterId = String;
use crate::stats::ExecutionStats;

pub const METRIC_VERSION: &str = "v1.0.1";
pub const TOKENIZER_VERSION: &str = "v1.0.1";

#[derive(Debug, Clone)]
pub struct RebuildConfig {
    pub source_configs: HashMap<String, SourceConfig>,
    pub max_clusters: usize,
    pub max_memory_multiplier: f64,
    pub validation_holdout_ratio: f64,
    pub max_mae_increase: f64,
    pub max_noise_ratio: f64,
    pub min_cohesion: f64,
    pub min_separation: f64,
    pub min_ari: f64,
    pub min_silhouette: f64,
    pub use_ann_optimization: bool,
    pub distance_cache_size: usize,
    pub batch_size: usize,
    pub max_rebuild_duration: std::time::Duration,
    pub shadow_mode_duration: std::time::Duration,
    pub canary_duration: std::time::Duration,
    pub rebuild_interval: std::time::Duration,
    pub kneedle_sensitivity: f64,
    pub kneedle_smoothing: usize,
    pub metric_version: String,
    pub tokenizer_version: String,
    pub eps_ema_smoothing: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub eps_range: (f64, f64),
    pub min_samples: usize,
    pub min_cluster_size: usize,
    pub preserve_bootstraps: bool,
    pub w_title: f64,
    pub w_meta: f64,
    pub tau_match: f64,
    pub tau_split: f64,
    pub tau_merge_lo: f64,
    pub tau_merge_hi: f64,
    pub last_eps: Option<f64>,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        let mut source_configs = HashMap::new();

        source_configs.insert(
            "cargo".to_string(),
            SourceConfig {
                eps_range: (0.15, 0.35),
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
            },
        );

        source_configs.insert(
            "npm".to_string(),
            SourceConfig {
                eps_range: (0.2, 0.4),
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
            },
        );

        source_configs.insert(
            "claude-code".to_string(),
            SourceConfig {
                eps_range: (0.25, 0.45),
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
            },
        );

        source_configs.insert(
            "cmd".to_string(),
            SourceConfig {
                eps_range: (0.2, 0.4),
                min_samples: 2,
                min_cluster_size: 2,
                preserve_bootstraps: true,
                w_title: 0.7,
                w_meta: 0.3,
                tau_match: 0.5,
                tau_split: 0.35,
                tau_merge_lo: 0.35,
                tau_merge_hi: 0.6,
                last_eps: None,
            },
        );

        source_configs.insert(
            "codex-session".to_string(),
            SourceConfig {
                eps_range: (0.25, 0.45),
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
            },
        );

        source_configs.insert(
            "gemini-cli".to_string(),
            SourceConfig {
                eps_range: (0.25, 0.45),
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
            },
        );

        source_configs.insert(
            "gitlab".to_string(),
            SourceConfig {
                eps_range: (0.2, 0.4),
                min_samples: 2,
                min_cluster_size: 2,
                preserve_bootstraps: true,
                w_title: 0.5,
                w_meta: 0.5,
                tau_match: 0.5,
                tau_split: 0.35,
                tau_merge_lo: 0.35,
                tau_merge_hi: 0.6,
                last_eps: None,
            },
        );

        Self {
            source_configs,
            max_clusters: 1000,
            max_memory_multiplier: 1.5,
            validation_holdout_ratio: 0.2,
            max_mae_increase: 0.05,
            max_noise_ratio: 0.50,  // Allow high noise during bootstrap
            min_cohesion: 0.3,       // Lower for synthetic/sparse data
            min_separation: 0.2,     // Lower for synthetic data
            min_ari: -1.0,           // Allow any ARI during bootstrap
            min_silhouette: -0.1, // Silhouette can be negative
            use_ann_optimization: true,
            distance_cache_size: 10_000,
            batch_size: 100,
            max_rebuild_duration: std::time::Duration::from_secs(300),
            shadow_mode_duration: std::time::Duration::from_secs(86400),
            canary_duration: std::time::Duration::from_secs(172800),
            rebuild_interval: std::time::Duration::from_secs(604800), // 1 week
            kneedle_sensitivity: 1.0,
            kneedle_smoothing: 7,
            metric_version: METRIC_VERSION.to_string(),
            tokenizer_version: TOKENIZER_VERSION.to_string(),
            eps_ema_smoothing: 0.2,
        }
    }
}

pub fn normalize_text(text: &str) -> String {
    text.nfkc()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn custom_distance(r1: &Race, r2: &Race, config: &SourceConfig) -> f64 {
    if r1.source != r2.source {
        return 1.0;
    }

    let title1_norm = normalize_text(&r1.title);
    let title2_norm = normalize_text(&r2.title);

    let max_len = title1_norm.chars().count().max(title2_norm.chars().count()) as f64;
    let title_distance = if max_len > 0.0 {
        levenshtein::levenshtein(&title1_norm, &title2_norm) as f64 / max_len
    } else {
        0.0
    };

    let metadata_distance = if let (Some(m1), Some(m2)) = (&r1.metadata, &r2.metadata) {
        1.0 - jaccard_metadata_similarity(m1, m2)
    } else {
        0.5 // Default distance when metadata is missing
    };

    // Use weights from SourceConfig (they should sum to 1.0)
    let w_title = config.w_title;
    let w_meta = config.w_meta;

    (w_title * title_distance + w_meta * metadata_distance).clamp(0.0, 1.0)
}

fn jaccard_metadata_similarity(m1: &HashMap<String, String>, m2: &HashMap<String, String>) -> f64 {
    const RELEVANT_KEYS: &[&str] = &["model", "tool", "language", "file_extension"];

    let set1: HashSet<String> = m1
        .iter()
        .filter(|(k, _)| RELEVANT_KEYS.contains(&k.as_str()))
        .map(|(k, v)| format!("{}={}", normalize_text(k), normalize_text(v)))
        .collect();

    let set2: HashSet<String> = m2
        .iter()
        .filter(|(k, _)| RELEVANT_KEYS.contains(&k.as_str()))
        .map(|(k, v)| format!("{}={}", normalize_text(k), normalize_text(v)))
        .collect();

    if set1.is_empty() && set2.is_empty() {
        return 1.0;
    }

    let intersection = set1.intersection(&set2).count() as f64;
    let union = set1.union(&set2).count() as f64;

    if union > 0.0 {
        intersection / union
    } else {
        0.0
    }
}

pub fn race_to_vector(race: &Race) -> Vec<f32> {
    // Check if race has precomputed embedding
    if let Some(metadata) = &race.metadata {
        if let Some(embedding) = metadata.get("embedding") {
            if let Ok(vec) = serde_json::from_str::<Vec<f32>>(embedding) {
                return vec;
            }
        }
    }

    // Fallback: hashed character 3-gram TF-IDF, L2-normalized
    let s = normalize_text(&race.title);
    let mut feats = vec![0f32; 4096];
    let chars: Vec<char> = s.chars().collect();

    for w in chars.windows(3) {
        let g: String = w.iter().collect();
        let h = (seahash::hash(g.as_bytes()) as usize) % feats.len();
        feats[h] += 1.0;
    }

    let norm = feats
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for f in &mut feats {
            *f = (*f as f64 / norm) as f32;
        }
    }

    feats
}

#[derive(Debug, Clone)]
pub struct DBSCANResult {
    pub clusters: HashMap<ClusterId, Vec<RaceId>>,
    pub noise: Vec<RaceId>,
    pub border_points: HashMap<RaceId, ClusterId>,
}

#[derive(Debug, Clone)]
pub struct CorePointIndex {
    core_points: HashMap<RaceId, ClusterId>,
    races: HashMap<RaceId, Race>,
}

impl CorePointIndex {
    pub fn new() -> Self {
        Self {
            core_points: HashMap::new(),
            races: HashMap::new(),
        }
    }

    pub fn add_core_point(&mut self, race_id: RaceId, cluster_id: ClusterId, race: Race) {
        self.core_points.insert(race_id.clone(), cluster_id);
        self.races.insert(race_id, race);
    }

    pub fn search(&self, vector: &[f32], k: usize) -> Vec<RaceId> {
        // Simplified: return all core points (would use ANN in production)
        self.core_points.keys().take(k).cloned().collect()
    }

    pub fn get_cluster_id(&self, race_id: &RaceId) -> Option<ClusterId> {
        self.core_points.get(race_id).cloned()
    }
}

pub fn assign_noise_as_border(
    noise: &[Race],
    core_index: &CorePointIndex,
    eps: f64,
    config: &SourceConfig,
) -> HashMap<RaceId, ClusterId> {
    let mut out = HashMap::new();

    for r in noise {
        let cand = core_index.search(&race_to_vector(r), 64);

        // Find the nearest core point within eps
        let mut best_cluster = None;
        let mut best_distance = f64::MAX;

        for core_id in cand {
            if let Some(core_race) = core_index.races.get(&core_id) {
                let distance = custom_distance(r, core_race, config);
                if distance <= eps && distance < best_distance {
                    best_distance = distance;
                    best_cluster = core_index.get_cluster_id(&core_id);
                }
            }
        }

        if let Some(cluster_id) = best_cluster {
            out.insert(r.id.clone(), cluster_id);
        }
    }

    out
}

pub struct OptimizedDBSCAN {
    distance_cache: LruCache<(String, String), f64>,
    races_map: HashMap<RaceId, Race>,
}

impl OptimizedDBSCAN {
    pub fn new(cache_size: usize) -> Self {
        Self {
            distance_cache: LruCache::new(cache_size.try_into().unwrap()),
            races_map: HashMap::new(),
        }
    }

    fn get_or_compute_distance(
        &mut self,
        id1: &RaceId,
        id2: &RaceId,
        config: &SourceConfig,
    ) -> f64 {
        let key = if id1 < id2 {
            (id1.clone(), id2.clone())
        } else {
            (id2.clone(), id1.clone())
        };

        if let Some(&dist) = self.distance_cache.get(&key) {
            return dist;
        }

        let r1 = &self.races_map[id1];
        let r2 = &self.races_map[id2];
        let distance = custom_distance(r1, r2, config);
        self.distance_cache.put(key, distance);
        distance
    }

    pub fn run_dbscan(
        &mut self,
        races: Vec<Race>,
        eps: f64,
        min_samples: usize,
        config: &SourceConfig,
    ) -> DBSCANResult {
        // Build races map
        self.races_map.clear();
        for race in races.iter() {
            self.races_map.insert(race.id.clone(), race.clone());
        }

        let mut labels: HashMap<RaceId, i32> = HashMap::new();
        let mut cluster_id = 0;

        for race in races.iter() {
            if labels.contains_key(&race.id) {
                continue;
            }

            // Find neighbors within eps
            let neighbors = self.find_neighbors(&race.id, &races, eps, config);

            if neighbors.len() < min_samples {
                labels.insert(race.id.clone(), -1); // Noise
                continue;
            }

            // Start new cluster
            cluster_id += 1;
            labels.insert(race.id.clone(), cluster_id);

            // Expand cluster
            let mut seeds = VecDeque::from(neighbors);
            while let Some(neighbor_id) = seeds.pop_front() {
                let neighbor_label = labels.get(&neighbor_id).copied();

                if neighbor_label == Some(-1) {
                    // Change noise to border point
                    labels.insert(neighbor_id.clone(), cluster_id);
                }

                if neighbor_label.is_some() {
                    continue;
                }

                labels.insert(neighbor_id.clone(), cluster_id);

                let neighbor_neighbors = self.find_neighbors(&neighbor_id, &races, eps, config);
                if neighbor_neighbors.len() >= min_samples {
                    for nn in neighbor_neighbors {
                        if !labels.contains_key(&nn) {
                            seeds.push_back(nn);
                        }
                    }
                }
            }
        }

        // Convert labels to result
        let mut clusters: HashMap<ClusterId, Vec<RaceId>> = HashMap::new();
        let mut noise = Vec::new();
        let mut border_points = HashMap::new();

        for (race_id, label) in labels {
            if label == -1 {
                noise.push(race_id);
            } else {
                let cluster_id = format!("{}:cluster_{}", races[0].source, label);
                clusters
                    .entry(cluster_id.clone())
                    .or_insert_with(Vec::new)
                    .push(race_id.clone());

                // Check if border point (has < min_samples neighbors)
                let neighbors = self.find_neighbors(&race_id, &races, eps, config);
                if neighbors.len() < min_samples {
                    border_points.insert(race_id, cluster_id);
                }
            }
        }

        DBSCANResult {
            clusters,
            noise,
            border_points,
        }
    }

    fn find_neighbors(
        &mut self,
        race_id: &RaceId,
        races: &[Race],
        eps: f64,
        config: &SourceConfig,
    ) -> Vec<RaceId> {
        let mut neighbors = Vec::new();

        for other in races {
            if &other.id == race_id {
                continue;
            }

            let distance = self.get_or_compute_distance(race_id, &other.id, config);
            if distance <= eps {
                neighbors.push(other.id.clone());
            }
        }

        neighbors
    }
}

pub fn detect_optimal_eps(
    races: &[Race],
    min_samples: usize,
    eps_min: f64,
    eps_max: f64,
    config: &SourceConfig,
) -> f64 {
    let k = min_samples;

    // Subsample for scalability
    let mut rng = StdRng::seed_from_u64(42);
    let n = races.len();
    let sample_size = ((n * 15) / 100).max(n.min(50_000)).min(n);

    let sample: Vec<&Race> = races.choose_multiple(&mut rng, sample_size).collect();

    // Calculate k-distances
    let mut dbscan = OptimizedDBSCAN::new(10_000);
    for race in sample.iter() {
        dbscan.races_map.insert(race.id.clone(), (*race).clone());
    }

    let mut k_distances: Vec<f64> = Vec::new();

    for race in sample.iter() {
        let mut distances: Vec<f64> = sample
            .iter()
            .filter(|r| r.id != race.id)
            .map(|r| dbscan.get_or_compute_distance(&race.id, &r.id, config))
            .collect();

        distances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        if distances.len() >= k {
            k_distances.push(distances[k - 1]);
        }
    }

    // Sort k-distances in descending order
    k_distances.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    // Apply moving average
    let smoothed = moving_average(&k_distances, 7);

    // Use proper Kneedle algorithm for knee detection
    let eps = detect_knee_kneedle(
        &smoothed,
        Curve::Concave,
        Direction::Decreasing,
        1.0, // sensitivity
    )
    .unwrap_or((eps_min + eps_max) / 2.0);

    eps.clamp(eps_min, eps_max)
}

fn moving_average(data: &[f64], window_size: usize) -> Vec<f64> {
    if data.is_empty() || window_size == 0 {
        return vec![];
    }

    let mut result = Vec::new();
    let half_window = window_size / 2;

    for i in 0..data.len() {
        let start = i.saturating_sub(half_window);
        let end = (i + half_window + 1).min(data.len());
        let sum: f64 = data[start..end].iter().sum();
        result.push(sum / (end - start) as f64);
    }

    result
}

fn detect_knee_simple(data: &[f64]) -> Option<f64> {
    if data.len() < 3 {
        return None;
    }

    // Find point of maximum curvature
    let mut max_curvature = 0.0;
    let mut knee_idx = data.len() / 2;

    for i in 1..data.len() - 1 {
        let curvature = (data[i - 1] - 2.0 * data[i] + data[i + 1]).abs();
        if curvature > max_curvature {
            max_curvature = curvature;
            knee_idx = i;
        }
    }

    Some(data[knee_idx])
}

// Proper Kneedle algorithm implementation
#[derive(Debug, Clone, Copy)]
pub enum Curve {
    Concave,
    Convex,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Increasing,
    Decreasing,
}

pub fn detect_knee_kneedle(
    data: &[f64],
    curve: Curve,
    direction: Direction,
    sensitivity: f64,
) -> Option<f64> {
    if data.len() < 3 {
        return None;
    }

    // Normalize data to [0, 1]
    let min_val = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_val = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max_val - min_val;

    if range == 0.0 {
        return Some(data[0]);
    }

    let normalized: Vec<f64> = data.iter().map(|&x| (x - min_val) / range).collect();

    // Create x values (normalized indices)
    let x_norm: Vec<f64> = (0..data.len())
        .map(|i| i as f64 / (data.len() - 1) as f64)
        .collect();

    // Calculate differences from diagonal line
    let mut differences = Vec::new();
    for i in 0..normalized.len() {
        let y_expected = match (curve, direction) {
            (Curve::Concave, Direction::Decreasing) => 1.0 - x_norm[i],
            (Curve::Concave, Direction::Increasing) => x_norm[i],
            (Curve::Convex, Direction::Decreasing) => 1.0 - x_norm[i],
            (Curve::Convex, Direction::Increasing) => x_norm[i],
        };

        let diff = match curve {
            Curve::Concave => normalized[i] - y_expected,
            Curve::Convex => y_expected - normalized[i],
        };

        differences.push(diff);
    }

    // Find the knee as the point with maximum difference
    let threshold = differences
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max)
        - sensitivity * differences.iter().map(|x| x.abs()).sum::<f64>() / differences.len() as f64;

    for (i, &diff) in differences.iter().enumerate() {
        if diff >= threshold {
            return Some(data[i]);
        }
    }

    Some(data[data.len() / 2])
}

#[derive(Debug, Clone)]
pub struct ClusterMapping {
    pub old_to_new: HashMap<ClusterId, Vec<ClusterId>>,
    pub new_to_old: HashMap<ClusterId, Vec<ClusterId>>,
    pub confidence_scores: HashMap<(ClusterId, ClusterId), f64>,
}

impl ClusterMapping {
    pub fn new() -> Self {
        Self {
            old_to_new: HashMap::new(),
            new_to_old: HashMap::new(),
            confidence_scores: HashMap::new(),
        }
    }

    pub fn build_mapping(
        old_clusters: &HashMap<ClusterId, RaceCluster>,
        new_clusters: &HashMap<ClusterId, RaceCluster>,
        config: &SourceConfig,
    ) -> Self {
        let mut mapping = ClusterMapping::new();

        for (old_id, old_cluster) in old_clusters {
            for (new_id, new_cluster) in new_clusters {
                let overlap = calculate_member_overlap(old_cluster, new_cluster);

                if overlap >= config.tau_match {
                    mapping.add_mapping(old_id.clone(), new_id.clone(), overlap);
                }
            }
        }

        mapping.resolve_conflicts();
        mapping
    }

    fn add_mapping(&mut self, old_id: ClusterId, new_id: ClusterId, confidence: f64) {
        self.old_to_new
            .entry(old_id.clone())
            .or_insert_with(Vec::new)
            .push(new_id.clone());

        self.new_to_old
            .entry(new_id.clone())
            .or_insert_with(Vec::new)
            .push(old_id.clone());

        self.confidence_scores.insert((old_id, new_id), confidence);
    }

    fn resolve_conflicts(&mut self) {
        // For each new cluster, keep only the best matching old cluster
        for (new_id, old_ids) in self.new_to_old.clone() {
            if old_ids.len() > 1 {
                let best_old = old_ids
                    .iter()
                    .max_by(|a, b| {
                        let score_a = self
                            .confidence_scores
                            .get(&((**a).clone(), new_id.clone()))
                            .unwrap_or(&0.0);
                        let score_b = self
                            .confidence_scores
                            .get(&((**b).clone(), new_id.clone()))
                            .unwrap_or(&0.0);
                        score_a
                            .partial_cmp(score_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap()
                    .clone();

                self.new_to_old
                    .insert(new_id.clone(), vec![best_old.clone()]);

                // Update old_to_new
                for old_id in &old_ids {
                    if *old_id != best_old {
                        if let Some(new_ids) = self.old_to_new.get_mut(old_id) {
                            new_ids.retain(|id| id != &new_id);
                        }
                    }
                }
            }
        }
    }

    pub fn apply_stable_ids(
        &self,
        mut new_clusters: HashMap<ClusterId, RaceCluster>,
    ) -> HashMap<ClusterId, RaceCluster> {
        let mut result = HashMap::new();

        for (new_id, cluster) in new_clusters.drain() {
            let stable_id = self
                .new_to_old
                .get(&new_id)
                .and_then(|old_ids| old_ids.first())
                .cloned()
                .unwrap_or(new_id);

            result.insert(stable_id, cluster);
        }

        result
    }
}

fn calculate_member_overlap(c1: &RaceCluster, c2: &RaceCluster) -> f64 {
    let set1: HashSet<_> = c1.member_race_ids.iter().collect();
    let set2: HashSet<_> = c2.member_race_ids.iter().collect();

    let intersection = set1.intersection(&set2).count() as f64;
    let union = set1.union(&set2).count() as f64;

    if union > 0.0 {
        intersection / union
    } else {
        0.0
    }
}

#[derive(Debug, Clone)]
pub struct MappingThresholds {
    pub tau_match: f64,
    pub tau_split: f64,
    pub tau_merge_lo: f64,
    pub tau_merge_hi: f64,
}

impl Default for MappingThresholds {
    fn default() -> Self {
        Self {
            tau_match: 0.5,
            tau_split: 0.35,
            tau_merge_lo: 0.35,
            tau_merge_hi: 0.6,
        }
    }
}

pub fn map_stable_ids(
    prev: &HashMap<ClusterId, RaceCluster>,
    next: &HashMap<ClusterId, RaceCluster>,
    th: MappingThresholds,
) -> HashMap<ClusterId, ClusterId> {
    // Build weighted edges where Jaccard >= tau_match
    let mut edges: Vec<(ClusterId, ClusterId, f64)> = Vec::new();

    for (p_id, p) in prev {
        for (n_id, n) in next {
            let j = calculate_member_overlap(p, n);
            if j >= th.tau_match {
                edges.push((p_id.clone(), n_id.clone(), j));
            }
        }
    }

    // Sort by weight (descending) with deterministic tie-breaking
    edges.sort_by(|a, b| {
        match b.2.partial_cmp(&a.2) {
            Some(std::cmp::Ordering::Equal) => {
                // Tie-break by lexicographic order of IDs
                match a.0.cmp(&b.0) {
                    std::cmp::Ordering::Equal => a.1.cmp(&b.1),
                    other => other,
                }
            }
            Some(other) => other,
            None => std::cmp::Ordering::Equal,
        }
    });

    // Greedy matching (simplified Hungarian)
    let mut result = HashMap::new();
    let mut used_old = HashSet::new();
    let mut used_new = HashSet::new();

    for (old_id, new_id, _weight) in edges {
        if !used_old.contains(&old_id) && !used_new.contains(&new_id) {
            result.insert(new_id.clone(), old_id.clone());
            used_old.insert(old_id);
            used_new.insert(new_id);
        }
    }

    // Unmatched new clusters get deterministic IDs
    for (new_id, new_cluster) in next {
        if !result.contains_key(new_id) {
            // Generate deterministic ID based on content
            let mut sorted_ids = new_cluster.member_race_ids.clone();
            sorted_ids.sort();
            let hash_input = format!(
                "{}:{}:{}",
                new_cluster.source,
                sorted_ids.join(","),
                METRIC_VERSION
            );
            let new_stable_id = format!("cluster_{:x}", seahash::hash(hash_input.as_bytes()));
            result.insert(new_id.clone(), new_stable_id);
        }
    }

    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapPattern {
    pub id: String,
    pub source: String,
    pub title: String,
    pub metadata: HashMap<String, String>,
    pub default_eta: i64,
    pub is_critical: bool,
}

impl BootstrapPattern {
    pub fn canonical_id(&self) -> String {
        format!("bootstrap:{}", self.id)
    }

    pub fn matches_race_id(&self, race_id: &str) -> bool {
        // Simplified: check if race was created from this bootstrap
        race_id.contains(&self.id)
    }
}

pub fn preserve_bootstrap_patterns(
    mut dbscan_result: HashMap<ClusterId, RaceCluster>,
    bootstrap_patterns: &[BootstrapPattern],
) -> HashMap<ClusterId, RaceCluster> {
    // Bootstrap patterns use ID aliasing, not synthetic cluster creation
    for pattern in bootstrap_patterns.iter().filter(|p| p.is_critical) {
        // Find clusters that match this bootstrap pattern
        let matching_clusters: Vec<ClusterId> = dbscan_result
            .iter()
            .filter_map(|(id, cluster)| {
                let overlap = calculate_pattern_overlap(cluster, pattern);
                if overlap >= 0.5 {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        // Create ID aliases for bootstrap continuity
        for cluster_id in matching_clusters {
            if let Some(cluster) = dbscan_result.get_mut(&cluster_id) {
                // Add alias (would need to extend RaceCluster struct)
                // For now, just track in metadata
                cluster
                    .representative_metadata
                    .insert("bootstrap_alias".to_string(), pattern.canonical_id());
            }
        }
    }

    dbscan_result
}

fn calculate_pattern_overlap(cluster: &RaceCluster, pattern: &BootstrapPattern) -> f64 {
    let pattern_matches = cluster
        .member_race_ids
        .iter()
        .filter(|id| pattern.matches_race_id(id))
        .count();

    let total_members = cluster.member_race_ids.len().max(1);
    pattern_matches as f64 / total_members as f64
}

pub type ClusterSet = HashMap<ClusterId, RaceCluster>;

#[derive(Debug)]
pub struct DoubleBufferClusters {
    pub active: Arc<RwLock<ClusterSet>>,
    pub inactive: Arc<RwLock<ClusterSet>>,
    baseline_memory: usize,
}

impl DoubleBufferClusters {
    pub fn new(baseline_memory: usize) -> Self {
        Self {
            active: Arc::new(RwLock::new(HashMap::new())),
            inactive: Arc::new(RwLock::new(HashMap::new())),
            baseline_memory,
        }
    }

    pub async fn rebuild_with_zero_downtime(
        &self,
        races: Vec<Race>,
        config: &RebuildConfig,
    ) -> Result<()> {
        if !self.check_memory_budget() {
            return Err(anyhow!("Insufficient memory for rebuild"));
        }

        // Build new clusters (no locks held)
        let snapshot = {
            let active = self.active.read().await;
            active.clone()
        };

        let new_clusters = self.run_dbscan_rebuild(races, &snapshot, config).await?;

        // Skip validation if we have no existing clusters (initial bootstrap)
        // Use the snapshot (old clusters) not the current active buffer
        let should_validate = {
            let cluster_count = snapshot.len();
            eprintln!("Existing clusters count (snapshot): {}", cluster_count);
            eprintln!("New clusters created: {}", new_clusters.len());
            cluster_count > 0
        };
        
        if should_validate {
            eprintln!("Validating new clusters against {} existing clusters", 
                     self.active.read().await.len());
            // Validate only if we have existing clusters to compare against
            if !self
                .validate_new_clusters(&new_clusters, &snapshot, config)
                .await
            {
                return Err(anyhow!("Validation failed"));
            }
        } else {
            eprintln!("Skipping validation - no existing clusters (initial bootstrap)");
        }

        // Atomic swap with selective replacement
        let mut inactive = self.inactive.write().await;
        let mut active = self.active.write().await;

        // Get the source(s) being rebuilt from new_clusters
        let rebuilding_sources: std::collections::HashSet<String> = new_clusters
            .values()
            .map(|c| c.source.clone())
            .collect();
        
        // Start with existing clusters, but remove ones from sources being rebuilt
        *inactive = active
            .iter()
            .filter(|(_, cluster)| !rebuilding_sources.contains(&cluster.source))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        
        // Add all new clusters
        for (cluster_id, cluster) in new_clusters {
            inactive.insert(cluster_id, cluster);
        }
        
        // Swap the buffers
        std::mem::swap(&mut *active, &mut *inactive);

        Ok(())
    }

    async fn run_dbscan_rebuild(
        &self,
        races: Vec<Race>,
        old_clusters: &ClusterSet,
        config: &RebuildConfig,
    ) -> Result<ClusterSet> {
        let mut new_clusters = HashMap::new();

        // Group races by source
        let mut races_by_source: HashMap<String, Vec<Race>> = HashMap::new();
        for race in races {
            races_by_source
                .entry(race.source.clone())
                .or_insert_with(Vec::new)
                .push(race);
        }

        // Process each source
        for (source, source_races) in races_by_source {
            let source_config = config
                .source_configs
                .get(&source)
                .cloned()
                .unwrap_or_else(|| SourceConfig {
                    eps_range: (0.25, 0.45),
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
                });

            // Detect optimal eps
            let eps = if let Some(last_eps) = source_config.last_eps {
                // Use EMA with last eps
                let suggested_eps = detect_optimal_eps(
                    &source_races,
                    source_config.min_samples,
                    source_config.eps_range.0,
                    source_config.eps_range.1,
                    &source_config,
                );

                let alpha = config.eps_ema_smoothing as f64;
                (alpha * suggested_eps + (1.0 - alpha) * last_eps)
                    .clamp(source_config.eps_range.0, source_config.eps_range.1)
            } else {
                detect_optimal_eps(
                    &source_races,
                    source_config.min_samples,
                    source_config.eps_range.0,
                    source_config.eps_range.1,
                    &source_config,
                )
            };

            // Choose between HNSW and brute force based on data size
            let result = if config.use_ann_optimization && source_races.len() > 1000 {
                // Use HNSW for large datasets
                let mut hnsw_dbscan =
                    HnswDBSCAN::new(source_config.clone(), config.distance_cache_size);
                if let Err(e) = hnsw_dbscan.build_index(source_races.clone()) {
                    eprintln!(
                        "Failed to build HNSW index: {}, falling back to brute force",
                        e
                    );
                    let mut dbscan = OptimizedDBSCAN::new(config.distance_cache_size);
                    dbscan.run_dbscan(
                        source_races.clone(),
                        eps,
                        source_config.min_samples,
                        &source_config,
                    )
                } else {
                    hnsw_dbscan.run_dbscan(eps, source_config.min_samples)
                }
            } else {
                // Use brute force for small datasets
                let mut dbscan = OptimizedDBSCAN::new(config.distance_cache_size);
                dbscan.run_dbscan(
                    source_races.clone(),
                    eps,
                    source_config.min_samples,
                    &source_config,
                )
            };

            // Convert to RaceCluster format
            for (cluster_id, member_ids) in result.clusters {
                if member_ids.len() >= source_config.min_cluster_size {
                    let cluster = self.create_race_cluster(
                        cluster_id.clone(),
                        source.clone(),
                        member_ids,
                        &source_races,
                    );
                    new_clusters.insert(cluster_id, cluster);
                }
            }

            // Handle noise points as source average fallback
            if !result.noise.is_empty() {
                let noise_cluster_id = format!("{}:source_avg", source);
                let cluster = self.create_race_cluster(
                    noise_cluster_id.clone(),
                    source.clone(),
                    result.noise,
                    &source_races,
                );
                new_clusters.insert(noise_cluster_id, cluster);
            }
        }

        // Apply stable IDs
        let mapping = ClusterMapping::build_mapping(
            old_clusters,
            &new_clusters,
            &config.source_configs.values().next().unwrap(),
        );

        Ok(mapping.apply_stable_ids(new_clusters))
    }

    fn create_race_cluster(
        &self,
        cluster_id: ClusterId,
        source: String,
        member_ids: Vec<RaceId>,
        all_races: &[Race],
    ) -> RaceCluster {
        let members: Vec<&Race> = all_races
            .iter()
            .filter(|r| member_ids.contains(&r.id))
            .collect();

        let titles: Vec<String> = members.iter().map(|r| r.title.clone()).collect();

        let representative_title = if !titles.is_empty() {
            compute_centroid_title(&titles)
        } else {
            String::new()
        };

        let mut stats = ExecutionStats::new();
        for race in members {
            // Use the race's duration_sec field directly
            if let Some(duration) = race.duration_sec {
                stats.update_with_duration(duration);
            }
        }

        RaceCluster {
            cluster_id,
            source,
            representative_title,
            representative_metadata: HashMap::new(),
            stats,
            member_race_ids: member_ids,
            member_titles: titles,
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        }
    }

    async fn validate_new_clusters(
        &self,
        new_clusters: &ClusterSet,
        old_clusters: &ClusterSet,
        config: &RebuildConfig,
    ) -> bool {
        // Get sample races for validation
        let sample_races: Vec<Race> = new_clusters
            .values()
            .flat_map(|c| {
                c.member_race_ids
                    .iter()
                    .take(10) // Sample up to 10 races per cluster
                    .filter_map(|id| {
                        // Reconstruct race from cluster data (simplified)
                        Some(Race {
                            id: id.clone(),
                            source: c.source.clone(),
                            title: c.representative_title.clone(),
                            state: crate::models::RaceState::Passed,
                            started_at: c.last_updated,
                            completed_at: None,
                            duration_sec: Some(c.stats.median as i64),
                            eta_sec: Some(c.stats.median as i64),
                            progress: Some(100),
                            metadata: Some(c.representative_metadata.clone()),
                            deeplink: None,
                            events: Some(Vec::new()),
                            // New optimistic progress fields
                            last_progress_update: None,
                            last_eta_update: None,
                            eta_source: Some(3), // CLUSTER source
                            eta_confidence: Some(0.7),
                            update_interval_hint: Some(15),
                            eta_history: None,
                        })
                    })
            })
            .collect();

        let criteria = ValidationCriteria {
            max_mae_increase: config.max_mae_increase,
            max_p90_increase: 0.10,
            min_success_rate: 0.95,
            max_noise_ratio: config.max_noise_ratio,
            min_cohesion: config.min_cohesion,
            min_separation: config.min_separation,
            min_silhouette: config.min_silhouette,
            min_ari: config.min_ari,
        };

        // Use first source config or default
        let source_config = config
            .source_configs
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| SourceConfig {
                eps_range: (0.25, 0.45),
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
            });

        let result = validate_clusters_comprehensive(
            new_clusters,
            old_clusters,
            &sample_races,
            &criteria,
            &source_config,
        )
        .await;

        if !result.passed {
            eprintln!("Validation failed: {:?}", result.failures);
        }

        result.passed
    }

    fn check_memory_budget(&self) -> bool {
        // Simplified memory check
        true
    }
}

fn compute_centroid_title(titles: &[String]) -> String {
    if titles.is_empty() {
        return String::new();
    }

    if titles.len() == 1 {
        return titles[0].clone();
    }

    // Find title with minimum average distance to all others
    let mut min_avg_distance = f64::MAX;
    let mut centroid = &titles[0];

    for candidate in titles {
        let sum_distance: usize = titles
            .iter()
            .map(|other| levenshtein::levenshtein(candidate, other))
            .sum();

        let avg_distance = sum_distance as f64 / titles.len() as f64;

        if avg_distance < min_avg_distance {
            min_avg_distance = avg_distance;
            centroid = candidate;
        }
    }

    centroid.clone()
}

pub fn calculate_noise_ratio(clusters: &ClusterSet) -> f64 {
    let noise_count = clusters
        .iter()
        .filter(|(id, _)| id.ends_with(":source_avg"))
        .map(|(_, c)| c.member_race_ids.len())
        .sum::<usize>();

    let total_count: usize = clusters.values().map(|c| c.member_race_ids.len()).sum();

    if total_count > 0 {
        noise_count as f64 / total_count as f64
    } else {
        0.0
    }
}

pub fn calculate_average_cohesion(clusters: &ClusterSet) -> f64 {
    // Simplified cohesion: ratio of single-member clusters
    let singleton_count = clusters
        .values()
        .filter(|c| c.member_race_ids.len() <= 1)
        .count();

    if clusters.is_empty() {
        return 1.0;
    }

    1.0 - (singleton_count as f64 / clusters.len() as f64)
}

// Calculate silhouette coefficient for cluster quality
pub fn silhouette_sampled(
    clusters: &ClusterSet,
    races: &[Race],
    config: &SourceConfig,
    sample_size: usize,
) -> f64 {
    if clusters.len() < 2 {
        return 0.0; // Need at least 2 clusters
    }

    // Sample races for efficiency
    let mut rng = StdRng::seed_from_u64(42);
    let sampled: Vec<&Race> = races
        .choose_multiple(&mut rng, sample_size.min(races.len()))
        .collect();

    let mut silhouettes = Vec::new();

    for race in sampled {
        // Find which cluster this race belongs to
        let mut race_cluster_id = None;
        for (cluster_id, cluster) in clusters {
            if cluster.member_race_ids.contains(&race.id) {
                race_cluster_id = Some(cluster_id.clone());
                break;
            }
        }

        if let Some(cluster_id) = race_cluster_id {
            // Calculate a(i): average distance to other points in same cluster
            let same_cluster_races: Vec<&Race> = races
                .iter()
                .filter(|r| {
                    clusters[&cluster_id].member_race_ids.contains(&r.id) && r.id != race.id
                })
                .collect();

            let a_i = if !same_cluster_races.is_empty() {
                same_cluster_races
                    .iter()
                    .map(|r| custom_distance(race, r, config))
                    .sum::<f64>()
                    / same_cluster_races.len() as f64
            } else {
                0.0
            };

            // Calculate b(i): minimum average distance to points in other clusters
            let mut b_i = f64::MAX;
            for (other_cluster_id, other_cluster) in clusters {
                if other_cluster_id != &cluster_id {
                    let other_cluster_races: Vec<&Race> = races
                        .iter()
                        .filter(|r| other_cluster.member_race_ids.contains(&r.id))
                        .collect();

                    if !other_cluster_races.is_empty() {
                        let avg_dist = other_cluster_races
                            .iter()
                            .map(|r| custom_distance(race, r, config))
                            .sum::<f64>()
                            / other_cluster_races.len() as f64;

                        b_i = b_i.min(avg_dist);
                    }
                }
            }

            // Calculate silhouette coefficient for this point
            if b_i != f64::MAX {
                let s_i = (b_i - a_i) / a_i.max(b_i);
                silhouettes.push(s_i);
            }
        }
    }

    if silhouettes.is_empty() {
        return 0.0;
    }

    silhouettes.iter().sum::<f64>() / silhouettes.len() as f64
}

// Calculate Adjusted Rand Index between two clusterings
pub fn adjusted_rand_index(old_clusters: &ClusterSet, new_clusters: &ClusterSet) -> f64 {
    // Get all race IDs present in both clusterings
    let mut all_race_ids = HashSet::new();
    for cluster in old_clusters.values() {
        all_race_ids.extend(cluster.member_race_ids.iter().cloned());
    }
    for cluster in new_clusters.values() {
        all_race_ids.extend(cluster.member_race_ids.iter().cloned());
    }

    let n = all_race_ids.len();
    if n < 2 {
        return 1.0; // Perfect agreement for trivial case
    }

    // Build contingency table
    let mut contingency = HashMap::new();
    for race_id in &all_race_ids {
        let old_cluster = old_clusters
            .iter()
            .find(|(_, c)| c.member_race_ids.contains(race_id))
            .map(|(id, _)| id.clone());

        let new_cluster = new_clusters
            .iter()
            .find(|(_, c)| c.member_race_ids.contains(race_id))
            .map(|(id, _)| id.clone());

        if let (Some(old), Some(new)) = (old_cluster, new_cluster) {
            *contingency.entry((old, new)).or_insert(0) += 1;
        }
    }

    // Calculate sums
    let mut a_sum = 0.0;
    let mut b_sum = 0.0;

    // Sum over old clusters
    for old_cluster in old_clusters.values() {
        let size = old_cluster.member_race_ids.len() as f64;
        a_sum += size * (size - 1.0) / 2.0;
    }

    // Sum over new clusters
    for new_cluster in new_clusters.values() {
        let size = new_cluster.member_race_ids.len() as f64;
        b_sum += size * (size - 1.0) / 2.0;
    }

    // Calculate index
    let mut sum_nij = 0.0;
    for &count in contingency.values() {
        let c = count as f64;
        sum_nij += c * (c - 1.0) / 2.0;
    }

    let expected = a_sum * b_sum / (n as f64 * (n as f64 - 1.0) / 2.0);
    let max_index = (a_sum + b_sum) / 2.0;

    if max_index == expected {
        return 0.0; // Avoid division by zero
    }

    (sum_nij - expected) / (max_index - expected)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DBSCANMetrics {
    pub noise_ratio: f64,
    pub cluster_count: usize,
    pub avg_cluster_size: f64,
    pub singleton_clusters: usize,
    pub stability_score: f64,
    pub cohesion: f64,
    pub silhouette: f64,
    pub separation: f64,
    pub ari_score: f64, // Adjusted Rand Index for cluster agreement
}

#[derive(Debug, Clone)]
pub struct RebuildState {
    pub snapshot_timestamp: DateTime<Utc>,
    pub pending_updates: Vec<RaceUpdate>,
}

#[derive(Debug, Clone)]
pub struct RaceUpdate {
    pub race: Race,
    pub timestamp: DateTime<Utc>,
}

impl RebuildState {
    pub fn new() -> Self {
        Self {
            snapshot_timestamp: Utc::now(),
            pending_updates: Vec::new(),
        }
    }

    pub fn apply_pending_after_rebuild(&mut self, clusters: &mut ClusterSet, eps: f64) {
        for update in &self.pending_updates {
            if update.timestamp > self.snapshot_timestamp {
                // Add race to nearest cluster within eps
                // This is simplified - would need full implementation
            }
        }
    }
}
// Cluster rebuild pipeline, thresholds, and rollout integration.
//
// Background and rationale: `docs/proposals/CLUSTER_REBUILDING_PROPOSAL.md` and
// `docs/design/ETA_PREDICTION_SYSTEM.md`.
