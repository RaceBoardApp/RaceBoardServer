use crate::stats::{EtaPrediction, ExecutionStats};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaceCluster {
    pub cluster_id: String,
    pub source: String,
    pub representative_title: String,
    pub representative_metadata: HashMap<String, String>,
    pub stats: ExecutionStats,
    pub member_race_ids: Vec<String>,
    pub member_titles: Vec<String>, // Store titles for centroid computation
    pub member_metadata_history: Vec<HashMap<String, String>>, // Store metadata history
    pub last_updated: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}

pub struct ClusteringEngine {
    pub clusters: Arc<RwLock<HashMap<String, RaceCluster>>>,
    pub similarity_threshold: f64,
    pub max_clusters: usize,
}

impl ClusteringEngine {
    pub fn new(max_clusters: usize) -> Self {
        Self {
            clusters: Arc::new(RwLock::new(HashMap::new())),
            similarity_threshold: 0.7,
            max_clusters,
        }
    }

    pub fn calculate_similarity(
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
        cluster: &RaceCluster,
    ) -> f64 {
        if race_source != cluster.source {
            return 0.0;
        }

        let title_similarity = 1.0
            - (levenshtein(race_title, &cluster.representative_title) as f64
                / race_title.len().max(cluster.representative_title.len()) as f64);

        let metadata_similarity =
            jaccard_similarity(race_metadata, &cluster.representative_metadata);

        (title_similarity * 0.6) + (metadata_similarity * 0.4)
    }

    pub async fn find_best_cluster(
        &self,
        _race_id: &str,
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
    ) -> Option<String> {
        let clusters = self.clusters.read().await;

        let mut best_match: Option<(String, f64)> = None;

        for (cluster_id, cluster) in clusters.iter() {
            let similarity =
                Self::calculate_similarity(race_title, race_source, race_metadata, cluster);

            if similarity >= self.similarity_threshold {
                match best_match {
                    None => best_match = Some((cluster_id.clone(), similarity)),
                    Some((_, best_score)) if similarity > best_score => {
                        best_match = Some((cluster_id.clone(), similarity))
                    }
                    _ => {}
                }
            }
        }

        best_match.map(|(id, _)| id)
    }

    pub async fn assign_race_to_cluster(
        &self,
        race_id: &str,
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
    ) -> String {
        if let Some(cluster_id) = self
            .find_best_cluster(race_id, race_title, race_source, race_metadata)
            .await
        {
            // Update existing cluster
            let mut clusters = self.clusters.write().await;
            if let Some(cluster) = clusters.get_mut(&cluster_id) {
                cluster.member_race_ids.push(race_id.to_string());
                if cluster.member_race_ids.len() > 100 {
                    cluster.member_race_ids.remove(0);
                }
                cluster.last_accessed = Utc::now();

                // Update representative every 10 members
                if cluster.member_race_ids.len() % 10 == 0 {
                    self.update_representative(cluster, race_title, race_metadata);
                }
            }
            cluster_id
        } else {
            // Create new cluster
            self.create_new_cluster(race_id, race_title, race_source, race_metadata)
                .await
        }
    }

    async fn create_new_cluster(
        &self,
        race_id: &str,
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
    ) -> String {
        let mut clusters = self.clusters.write().await;

        // Check if we need to evict
        if clusters.len() >= self.max_clusters {
            self.evict_lru_cluster(&mut clusters);
        }

        let operation_type = extract_operation_type(race_source, race_title, race_metadata);
        let cluster_id = format!("{}:{}", race_source, operation_type);

        let cluster = RaceCluster {
            cluster_id: cluster_id.clone(),
            source: race_source.to_string(),
            representative_title: race_title.to_string(),
            representative_metadata: race_metadata.clone(),
            stats: ExecutionStats::new(),
            member_race_ids: vec![race_id.to_string()],
            member_titles: vec![race_title.to_string()],
            member_metadata_history: vec![race_metadata.clone()],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        };

        clusters.insert(cluster_id.clone(), cluster);
        cluster_id
    }

    fn evict_lru_cluster(&self, clusters: &mut HashMap<String, RaceCluster>) {
        if let Some(lru_id) = clusters
            .iter()
            .min_by_key(|(_, c)| c.last_accessed)
            .map(|(id, _)| id.clone())
        {
            clusters.remove(&lru_id);
        }
    }

    fn update_representative(
        &self,
        cluster: &mut RaceCluster,
        new_title: &str,
        new_metadata: &HashMap<String, String>,
    ) {
        // Add the new title and metadata to history
        cluster.member_titles.push(new_title.to_string());
        cluster.member_metadata_history.push(new_metadata.clone());

        // Keep only recent history (last 50 members)
        const MAX_HISTORY: usize = 50;
        if cluster.member_titles.len() > MAX_HISTORY {
            cluster.member_titles =
                cluster.member_titles[cluster.member_titles.len() - MAX_HISTORY..].to_vec();
            cluster.member_metadata_history = cluster.member_metadata_history
                [cluster.member_metadata_history.len() - MAX_HISTORY..]
                .to_vec();
        }

        // Update representative every 10 members or if cluster is small
        if cluster.member_race_ids.len() % 10 == 0 || cluster.member_race_ids.len() <= 10 {
            // Compute centroid title: the one with minimum average distance to all others
            if !cluster.member_titles.is_empty() {
                let centroid_title = self.compute_centroid_title(&cluster.member_titles);
                cluster.representative_title = centroid_title;
            }

            // Compute most common metadata keys
            cluster.representative_metadata =
                self.compute_centroid_metadata(&cluster.member_metadata_history);
        }

        cluster.last_updated = Utc::now();
    }

    fn compute_centroid_title(&self, titles: &[String]) -> String {
        if titles.is_empty() {
            return String::new();
        }

        if titles.len() == 1 {
            return titles[0].clone();
        }

        // Find title with minimum average Levenshtein distance to all others
        let mut min_avg_distance = f64::MAX;
        let mut centroid_title = titles[0].clone();

        for candidate in titles {
            let total_distance: usize = titles
                .iter()
                .map(|other| levenshtein(candidate, other))
                .sum();

            let avg_distance = total_distance as f64 / titles.len() as f64;

            if avg_distance < min_avg_distance {
                min_avg_distance = avg_distance;
                centroid_title = candidate.clone();
            }
        }

        centroid_title
    }

    fn compute_centroid_metadata(
        &self,
        metadata_history: &[HashMap<String, String>],
    ) -> HashMap<String, String> {
        let mut key_value_counts: HashMap<(String, String), usize> = HashMap::new();
        let mut key_counts: HashMap<String, usize> = HashMap::new();

        // Count occurrences of each key-value pair
        for metadata in metadata_history {
            for (key, value) in metadata {
                *key_value_counts
                    .entry((key.clone(), value.clone()))
                    .or_insert(0) += 1;
                *key_counts.entry(key.clone()).or_insert(0) += 1;
            }
        }

        // Build representative metadata with most common values for each key
        let mut representative = HashMap::new();

        for (key, total_count) in key_counts {
            // Only include keys that appear in >50% of samples
            if total_count > metadata_history.len() / 2 {
                // Find most common value for this key
                let most_common_value = key_value_counts
                    .iter()
                    .filter(|((k, _), _)| k == &key)
                    .max_by_key(|(_, count)| *count)
                    .map(|((_, v), _)| v.clone());

                if let Some(value) = most_common_value {
                    representative.insert(key, value);
                }
            }
        }

        representative
    }

    pub async fn update_cluster_stats(&self, cluster_id: &str, duration: i64) {
        let mut clusters = self.clusters.write().await;
        if let Some(cluster) = clusters.get_mut(cluster_id) {
            cluster.stats.update_with_duration(duration);
            cluster.last_updated = Utc::now();
            cluster.last_accessed = Utc::now();
        }
    }

    pub async fn get_cluster_eta(&self, cluster_id: &str) -> Option<EtaPrediction> {
        let clusters = self.clusters.read().await;
        clusters.get(cluster_id).map(|c| c.stats.calculate_eta())
    }
}

fn levenshtein(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();
    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    for (i, c1) in s1.chars().enumerate() {
        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            matrix[i + 1][j + 1] = std::cmp::min(
                matrix[i][j + 1] + 1,
                std::cmp::min(matrix[i + 1][j] + 1, matrix[i][j] + cost),
            );
        }
    }

    matrix[len1][len2]
}

fn jaccard_similarity(set1: &HashMap<String, String>, set2: &HashMap<String, String>) -> f64 {
    use std::collections::HashSet;

    let set1_keys: HashSet<_> = set1.keys().collect();
    let set2_keys: HashSet<_> = set2.keys().collect();

    let intersection = set1_keys.intersection(&set2_keys).count();
    let union = set1_keys.union(&set2_keys).count();

    if union == 0 {
        return 1.0; // Both empty
    }

    intersection as f64 / union as f64
}

pub fn extract_operation_type(
    source: &str,
    title: &str,
    metadata: &HashMap<String, String>,
) -> String {
    match source {
        s if s.contains("claude") || s.contains("gemini") || s.contains("codex") => {
            let prompt = metadata.get("prompt").map(|s| s.as_str()).unwrap_or(title);

            if prompt.contains("implement") || prompt.contains("create") {
                "code_generation".to_string()
            } else if prompt.contains("debug") || prompt.contains("fix") {
                "debugging".to_string()
            } else if prompt.contains("explain") || prompt.contains("review") {
                "analysis".to_string()
            } else {
                "simple_prompt".to_string()
            }
        }
        "cargo" => {
            if title.contains("test") {
                "test_suite".to_string()
            } else if title.contains("clean") {
                "clean_build".to_string()
            } else {
                "incremental_build".to_string()
            }
        }
        "npm" => {
            if title.contains("install") {
                "install".to_string()
            } else if title.contains("build") {
                "build".to_string()
            } else {
                "default".to_string()
            }
        }
        _ => "default".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("saturday", "sunday"), 3);
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("a", ""), 1);
    }

    #[test]
    fn test_jaccard_similarity() {
        let mut set1 = HashMap::new();
        let mut set2 = HashMap::new();

        set1.insert("key1".to_string(), "value1".to_string());
        set1.insert("key2".to_string(), "value2".to_string());

        set2.insert("key2".to_string(), "value2".to_string());
        set2.insert("key3".to_string(), "value3".to_string());

        let similarity = jaccard_similarity(&set1, &set2);
        assert!((similarity - 0.333).abs() < 0.01);
    }

    #[test]
    fn test_extract_operation_type() {
        let mut metadata = HashMap::new();
        metadata.insert("prompt".to_string(), "implement a function".to_string());

        assert_eq!(
            extract_operation_type("claude-code", "", &metadata),
            "code_generation"
        );

        assert_eq!(
            extract_operation_type("cargo", "cargo test", &HashMap::new()),
            "test_suite"
        );
    }
}
