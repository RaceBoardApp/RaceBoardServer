use crate::cluster::{extract_operation_type, ClusteringEngine};
use crate::persistence::PersistenceLayer;
use crate::stats::{EtaPrediction, ExecutionStats};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PredictionEngine {
    pub clustering_engine: Arc<ClusteringEngine>,
    persistence: Arc<PersistenceLayer>,
    source_stats: Arc<RwLock<HashMap<String, SourceStats>>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceStats {
    pub source: String,
    pub execution_history: VecDeque<i64>,
    pub stats: ExecutionStats,
    pub last_updated: DateTime<Utc>,
    pub max_history_size: usize,
}

impl PredictionEngine {
    pub fn new(
        clustering_engine: Arc<ClusteringEngine>,
        persistence: Arc<PersistenceLayer>,
    ) -> Self {
        // Load existing source stats from persistence
        let mut initial_stats = HashMap::new();
        if let Ok(saved_stats) = persistence.load_source_stats() {
            initial_stats = saved_stats;
            log::info!("Loaded {} source statistics from disk", initial_stats.len());
        }

        Self {
            clustering_engine,
            persistence,
            source_stats: Arc::new(RwLock::new(initial_stats)),
        }
    }

    pub async fn predict_eta(
        &self,
        race_id: &str,
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
    ) -> EtaPrediction {
        // Level 1: Try to find best matching cluster
        if let Some(cluster_id) = self
            .clustering_engine
            .find_best_cluster(race_id, race_title, race_source, race_metadata)
            .await
        {
            if let Some(prediction) = self.clustering_engine.get_cluster_eta(&cluster_id).await {
                if prediction.confidence > 0.3 {
                    return prediction;
                }
            }
        }

        // Level 2: Source-level statistics
        let source_stats = self.source_stats.read().await;
        if let Some(source_stat) = source_stats.get(race_source) {
            if source_stat.execution_history.len() >= 5 {
                let prediction = source_stat.stats.calculate_eta();
                // Boost confidence slightly for source-level stats
                return EtaPrediction {
                    expected_seconds: prediction.expected_seconds,
                    confidence: (prediction.confidence * 0.7).min(0.6),
                    lower_bound: prediction.lower_bound,
                    upper_bound: prediction.upper_bound,
                };
            }
        }
        drop(source_stats);

        // Level 3: Bootstrap defaults
        let default_eta = self
            .get_bootstrap_default(race_source, race_title, race_metadata)
            .await;

        EtaPrediction {
            expected_seconds: default_eta,
            confidence: 0.2,
            lower_bound: (default_eta as f64 * 0.5) as i64,
            upper_bound: (default_eta as f64 * 2.0) as i64,
        }
    }

    async fn get_bootstrap_default(
        &self,
        source: &str,
        title: &str,
        metadata: &HashMap<String, String>,
    ) -> i64 {
        let operation = extract_operation_type(source, title, metadata);

        // Match against bootstrap patterns
        match (source, operation.as_str()) {
            ("claude-code", "simple_prompt") => 15,
            ("claude-code", "code_generation") => 30,
            ("claude-code", "complex_analysis") => 45,
            ("claude-code", _) => 20,

            ("gemini-cli", "simple_prompt") => 10,
            ("gemini-cli", "code_generation") => 25,
            ("gemini-cli", _) => 15,

            ("codex", "simple_prompt") => 20,
            ("codex", "code_generation") => 35,
            ("codex", _) => 25,

            ("cargo", "incremental_build") => 5,
            ("cargo", "clean_build") => 60,
            ("cargo", "test_suite") => 30,
            ("cargo", _) => 15,

            ("npm", "install") => 30,
            ("npm", "build") => 45,
            ("npm", _) => 20,

            ("github-actions", "unit_tests") => 120,
            ("github-actions", "integration_tests") => 300,
            ("github-actions", _) => 180,

            ("jenkins", "deploy_staging") => 180,
            ("jenkins", "deploy_production") => 600,
            ("jenkins", _) => 300,

            _ => 30, // Ultimate fallback
        }
    }

    pub async fn update_source_stats(&self, source: &str, duration: i64) {
        let mut stats = self.source_stats.write().await;

        let entry = stats
            .entry(source.to_string())
            .or_insert_with(|| SourceStats {
                source: source.to_string(),
                execution_history: VecDeque::with_capacity(100),
                stats: ExecutionStats::new(),
                last_updated: Utc::now(),
                max_history_size: 100,
            });

        // Add to history (maintain max size)
        entry.execution_history.push_back(duration);
        if entry.execution_history.len() > entry.max_history_size {
            entry.execution_history.pop_front();
        }

        // Update statistics with full history
        entry.stats.update_with_duration(duration);
        entry.last_updated = Utc::now();
    }

    pub async fn on_race_completed(
        &self,
        race_id: &str,
        race_title: &str,
        race_source: &str,
        race_metadata: &HashMap<String, String>,
        duration: i64,
    ) {
        // Assign to cluster if not already assigned
        let cluster_id = self
            .clustering_engine
            .assign_race_to_cluster(race_id, race_title, race_source, race_metadata)
            .await;

        // Update cluster statistics
        self.clustering_engine
            .update_cluster_stats(&cluster_id, duration)
            .await;

        // Update source-level statistics with full history
        self.update_source_stats(race_source, duration).await;

        // Persist the updated cluster
        let clusters = self.clustering_engine.clusters.read().await;
        if let Some(cluster) = clusters.get(&cluster_id) {
            let _ = self.persistence.persist_cluster(cluster);
        }
        drop(clusters);

        // Also persist source stats periodically (every 10 updates)
        let source_stats = self.source_stats.read().await;
        if let Some(source_stat) = source_stats.get(race_source) {
            if source_stat.execution_history.len() % 10 == 0 {
                let _ = self
                    .persistence
                    .persist_source_stats(race_source, source_stat);
            }
        }
    }

    pub async fn get_source_stats(&self, source: &str) -> Option<SourceStats> {
        let stats = self.source_stats.read().await;
        stats.get(source).cloned()
    }

    pub async fn get_all_source_stats(&self) -> HashMap<String, SourceStats> {
        let stats = self.source_stats.read().await;
        stats.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bootstrap_defaults() {
        let clustering_engine = Arc::new(ClusteringEngine::new(100));
        let persistence = Arc::new(PersistenceLayer::new_in_memory().unwrap());
        let predictor = PredictionEngine::new(clustering_engine, persistence);

        let metadata = HashMap::new();

        let eta = predictor
            .get_bootstrap_default("cargo", "cargo build", &metadata)
            .await;
        assert_eq!(eta, 5); // incremental_build

        let eta = predictor
            .get_bootstrap_default("cargo", "cargo test", &metadata)
            .await;
        assert_eq!(eta, 30); // test_suite

        let eta = predictor
            .get_bootstrap_default("unknown", "something", &metadata)
            .await;
        assert_eq!(eta, 30); // fallback
    }
}

// Prediction engine for ETA and source statistics.
//
// See the high-level design in `docs/design/ETA_PREDICTION_SYSTEM.md` and how it
// integrates with the data layer in `docs/specs/DATA_LAYER_SPECIFICATION.md`.
