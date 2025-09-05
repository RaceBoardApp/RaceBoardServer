use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageHealth {
    pub total_races: usize,
    pub max_races: usize,
    pub usage_percent: f64,
    pub persistence_healthy: bool,
    pub last_eviction: Option<DateTime<Utc>>,
    pub eviction_count: usize,
    pub cluster_data_sufficient: bool,
    pub min_races_for_clustering: usize,
    pub races_by_source: std::collections::HashMap<String, usize>,
    pub warnings: Vec<String>,
    pub critical_errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MonitoringSystem {
    pub storage_health: Arc<RwLock<StorageHealth>>,
    eviction_count: Arc<RwLock<usize>>,
    last_eviction: Arc<RwLock<Option<DateTime<Utc>>>>,
}

impl MonitoringSystem {
    pub fn new(max_races: usize) -> Self {
        let initial_health = StorageHealth {
            total_races: 0,
            max_races,
            usage_percent: 0.0,
            persistence_healthy: true,
            last_eviction: None,
            eviction_count: 0,
            cluster_data_sufficient: false,
            min_races_for_clustering: 1000,
            races_by_source: std::collections::HashMap::new(),
            warnings: Vec::new(),
            critical_errors: Vec::new(),
        };

        Self {
            storage_health: Arc::new(RwLock::new(initial_health)),
            eviction_count: Arc::new(RwLock::new(0)),
            last_eviction: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn record_eviction(&self, race_id: &str) {
        let mut count = self.eviction_count.write().await;
        *count += 1;

        let mut last = self.last_eviction.write().await;
        *last = Some(Utc::now());

        log::error!(
            "DATA LOSS: Race {} evicted from storage. Total evictions: {}",
            race_id,
            *count
        );

        // Update health
        let mut health = self.storage_health.write().await;
        health.eviction_count = *count;
        health.last_eviction = *last;
        health.critical_errors.push(format!(
            "Race {} evicted at {} - DATA LOSS",
            race_id,
            Utc::now().to_rfc3339()
        ));
    }

    pub async fn update_storage_stats(
        &self,
        total_races: usize,
        races_by_source: std::collections::HashMap<String, usize>,
    ) {
        let mut health = self.storage_health.write().await;

        // Use in-memory counts for active races; historical counts are tracked separately via analytics
        health.total_races = total_races;
        health.usage_percent = (health.total_races as f64 / health.max_races as f64) * 100.0;
        health.races_by_source = races_by_source;

        // Clear previous warnings and errors
        health.warnings.clear();
        health.critical_errors.clear();

        // Check for issues
        let usage = health.usage_percent;
        if usage > 90.0 {
            health.warnings.push(format!(
                "Storage at {:.1}% capacity - data loss imminent!",
                usage
            ));
        } else if usage > 75.0 {
            health
                .warnings
                .push(format!("Storage at {:.1}% capacity", usage));
        }

        // Check cluster data sufficiency
        let mut sufficient_sources = 0;
        let mut new_warnings = Vec::new();

        for (source, count) in &health.races_by_source {
            if *count >= health.min_races_for_clustering {
                sufficient_sources += 1;
            } else if *count < 100 {
                new_warnings.push(format!(
                    "Source '{}' has only {} races - need 1000+ for accurate clustering",
                    source, count
                ));
            }
        }

        // Now append the warnings
        health.warnings.extend(new_warnings);
        health.cluster_data_sufficient = sufficient_sources > 0;

        if !health.cluster_data_sufficient {
            health
                .critical_errors
                .push("INSUFFICIENT DATA FOR CLUSTERING - Need 1000+ races per source".to_string());
        }

        // Log critical issues
        if health.usage_percent > 90.0 {
            log::error!(
                "CRITICAL: Storage at {:.1}% capacity!",
                health.usage_percent
            );
        }

        if health.eviction_count > 0 {
            log::error!(
                "CRITICAL: {} races have been evicted - DATA LOSS HAS OCCURRED",
                health.eviction_count
            );
        }
    }

    pub async fn check_initial_data(
        &self,
        storage: &crate::storage::Storage,
        persistence: &crate::persistence::PersistenceLayer,
    ) {
        // Count races from both in-memory and persisted storage
        let in_memory_races = storage.get_all_races().await;
        let mut races_by_source = std::collections::HashMap::new();

        // Count in-memory races
        for race in &in_memory_races {
            *races_by_source.entry(race.source.clone()).or_insert(0) += 1;
        }

        // Count persisted races
        let filter = crate::persistence::RaceScanFilter {
            source: None,
            from: None,
            to: None,
            include_events: false,
        };

        let mut cursor: Option<String> = None;
        loop {
            match persistence
                .scan_races(filter.clone(), 1000, cursor.clone())
                .await
            {
                Ok(batch) => {
                    if batch.items.is_empty() {
                        break;
                    }
                    for race in &batch.items {
                        *races_by_source.entry(race.source.clone()).or_insert(0) += 1;
                    }
                    cursor = batch.next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Failed to scan persisted races for initial check: {}", e);
                    break;
                }
            }
        }

        let total_races = races_by_source.values().sum();
        self.update_storage_stats(total_races, races_by_source)
            .await;
    }

    pub async fn check_persistence_health(
        &self,
        persistence: &crate::persistence::PersistenceLayer,
    ) {
        let mut health = self.storage_health.write().await;

        // Try to write and read a test value
        match persistence.get_db_size() {
            Ok(size) => {
                health.persistence_healthy = true;
                if size > 1_000_000_000 {
                    // 1GB
                    health.warnings.push(format!(
                        "Persistence database is large: {} MB",
                        size / 1_000_000
                    ));
                }
            }
            Err(e) => {
                health.persistence_healthy = false;
                health
                    .critical_errors
                    .push(format!("Persistence layer unhealthy: {}", e));
                log::error!("Persistence layer health check failed: {}", e);
            }
        }
    }

    pub async fn get_health(&self) -> StorageHealth {
        self.storage_health.read().await.clone()
    }

    pub async fn start_monitoring(
        self: Arc<Self>,
        storage: Arc<crate::storage::Storage>,
        persistence: Arc<crate::persistence::PersistenceLayer>,
    ) {
        let monitoring = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));

            loop {
                interval.tick().await;

                // Get current stats from BOTH in-memory and persisted storage
                let in_memory_races = storage.get_all_races().await;
                let mut races_by_source = std::collections::HashMap::new();

                // Count in-memory races
                for race in &in_memory_races {
                    *races_by_source.entry(race.source.clone()).or_insert(0) += 1;
                }

                // Also count persisted races
                // Get persisted race counts by source
                let filter = crate::persistence::RaceScanFilter {
                    source: None,
                    from: None,
                    to: None,
                    include_events: false,
                };

                // Scan in batches to count races by source
                let mut cursor: Option<String> = None;
                loop {
                    match persistence
                        .scan_races(filter.clone(), 1000, cursor.clone())
                        .await
                    {
                        Ok(batch) => {
                            if batch.items.is_empty() {
                                break;
                            }
                            for race in &batch.items {
                                *races_by_source.entry(race.source.clone()).or_insert(0) += 1;
                            }
                            cursor = batch.next_cursor;
                            if cursor.is_none() {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to scan persisted races for monitoring: {}", e);
                            break;
                        }
                    }
                }

                // Use total count from both sources
                let total_races = races_by_source.values().sum();
                monitoring
                    .update_storage_stats(total_races, races_by_source)
                    .await;

                // Log health summary
                let health = monitoring.get_health().await;

                if !health.critical_errors.is_empty() {
                    log::error!(
                        "Storage health critical errors: {:?}",
                        health.critical_errors
                    );
                }

                if !health.warnings.is_empty() {
                    log::warn!("Storage health warnings: {:?}", health.warnings);
                }

                log::info!(
                    "Storage health: {}/{} races ({:.1}%), {} evictions, clustering data: {}",
                    health.total_races,
                    health.max_races,
                    health.usage_percent,
                    health.eviction_count,
                    if health.cluster_data_sufficient {
                        "sufficient"
                    } else {
                        "INSUFFICIENT"
                    }
                );
            }
        });
    }
}

// Comprehensive metrics as specified in DATA_LAYER_SPECIFICATION v7
#[derive(Debug, Clone)]
pub struct DataLayerMetrics {
    // Gauges
    pub sled_db_size_bytes: Arc<AtomicU64>,
    pub tree_counts: Arc<RwLock<std::collections::HashMap<String, usize>>>,
    pub read_only_mode_active: Arc<RwLock<bool>>,

    // Counters
    pub flush_failures_total: Arc<AtomicUsize>,
    pub serialize_failures_total: Arc<AtomicUsize>,
    pub deserialize_failures_total: Arc<AtomicUsize>,
    pub json_snapshot_success_total: Arc<AtomicUsize>,
    pub purge_requests_total: Arc<AtomicUsize>,
    pub purge_failures_total: Arc<AtomicUsize>,

    // Histograms/Summaries
    pub write_latency_ms: Arc<RwLock<LatencyHistogram>>,
    pub flush_latency_ms: Arc<RwLock<LatencyHistogram>>,
    pub compaction_seconds_total: Arc<RwLock<Vec<f64>>>,

    // Timestamps
    pub compaction_last_success_timestamp: Arc<RwLock<Option<DateTime<Utc>>>>,
    pub json_snapshot_last_success_timestamp: Arc<RwLock<Option<DateTime<Utc>>>>,
}

#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    samples: Vec<f64>,
    max_samples: usize,
}

impl LatencyHistogram {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::with_capacity(max_samples),
            max_samples,
        }
    }

    pub fn record(&mut self, latency_ms: f64) {
        if self.samples.len() >= self.max_samples {
            self.samples.remove(0);
        }
        self.samples.push(latency_ms);
    }

    pub fn percentile(&self, p: f64) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64) as usize;
        Some(sorted[idx])
    }

    pub fn p50(&self) -> Option<f64> {
        self.percentile(50.0)
    }
    pub fn p95(&self) -> Option<f64> {
        self.percentile(95.0)
    }
    pub fn p99(&self) -> Option<f64> {
        self.percentile(99.0)
    }
}

impl DataLayerMetrics {
    pub fn new() -> Self {
        Self {
            sled_db_size_bytes: Arc::new(AtomicU64::new(0)),
            tree_counts: Arc::new(RwLock::new(std::collections::HashMap::new())),
            read_only_mode_active: Arc::new(RwLock::new(false)),
            flush_failures_total: Arc::new(AtomicUsize::new(0)),
            serialize_failures_total: Arc::new(AtomicUsize::new(0)),
            deserialize_failures_total: Arc::new(AtomicUsize::new(0)),
            json_snapshot_success_total: Arc::new(AtomicUsize::new(0)),
            purge_requests_total: Arc::new(AtomicUsize::new(0)),
            purge_failures_total: Arc::new(AtomicUsize::new(0)),
            write_latency_ms: Arc::new(RwLock::new(LatencyHistogram::new(10000))),
            flush_latency_ms: Arc::new(RwLock::new(LatencyHistogram::new(10000))),
            compaction_seconds_total: Arc::new(RwLock::new(Vec::new())),
            compaction_last_success_timestamp: Arc::new(RwLock::new(None)),
            json_snapshot_last_success_timestamp: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn record_write_latency(&self, latency_ms: f64) {
        let mut hist = self.write_latency_ms.write().await;
        hist.record(latency_ms);
    }

    pub async fn record_flush_latency(&self, latency_ms: f64) {
        let mut hist = self.flush_latency_ms.write().await;
        hist.record(latency_ms);
    }

    pub fn increment_flush_failures(&self) {
        self.flush_failures_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_serialize_failures(&self) {
        self.serialize_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_deserialize_failures(&self) {
        self.deserialize_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_snapshot_success(&self) {
        self.json_snapshot_success_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub async fn update_snapshot_timestamp(&self) {
        let mut ts = self.json_snapshot_last_success_timestamp.write().await;
        *ts = Some(Utc::now());
    }

    pub async fn get_metrics_summary(&self) -> serde_json::Value {
        let write_hist = self.write_latency_ms.read().await;
        let flush_hist = self.flush_latency_ms.read().await;

        serde_json::json!({
            "sled_db_size_bytes": self.sled_db_size_bytes.load(Ordering::Relaxed),
            "tree_counts": self.tree_counts.read().await.clone(),
            "read_only_mode_active": *self.read_only_mode_active.read().await,
            "flush_failures_total": self.flush_failures_total.load(Ordering::Relaxed),
            "serialize_failures_total": self.serialize_failures_total.load(Ordering::Relaxed),
            "deserialize_failures_total": self.deserialize_failures_total.load(Ordering::Relaxed),
            "json_snapshot_success_total": self.json_snapshot_success_total.load(Ordering::Relaxed),
            "purge_requests_total": self.purge_requests_total.load(Ordering::Relaxed),
            "purge_failures_total": self.purge_failures_total.load(Ordering::Relaxed),
            "write_latency_ms": {
                "p50": write_hist.p50(),
                "p95": write_hist.p95(),
                "p99": write_hist.p99(),
            },
            "flush_latency_ms": {
                "p50": flush_hist.p50(),
                "p95": flush_hist.p95(),
                "p99": flush_hist.p99(),
            },
            "compaction_last_success": self.compaction_last_success_timestamp.read().await.clone(),
            "snapshot_last_success": self.json_snapshot_last_success_timestamp.read().await.clone(),
        })
    }

    pub async fn check_slos(&self) -> Vec<String> {
        let mut violations = Vec::new();

        // Check write latency SLO (p95 ≤ 25ms)
        if let Some(p95) = self.write_latency_ms.read().await.p95() {
            if p95 > 25.0 {
                violations.push(format!(
                    "Write latency p95 ({:.2}ms) exceeds SLO (25ms)",
                    p95
                ));
            }
        }

        // Check flush latency SLO (p99 ≤ 200ms)
        if let Some(p99) = self.flush_latency_ms.read().await.p99() {
            if p99 > 200.0 {
                violations.push(format!(
                    "Flush latency p99 ({:.2}ms) exceeds SLO (200ms)",
                    p99
                ));
            }
        }

        // Check for flush failures
        if self.flush_failures_total.load(Ordering::Relaxed) > 0 {
            violations.push(format!(
                "Flush failures detected: {}",
                self.flush_failures_total.load(Ordering::Relaxed)
            ));
        }

        // Check for deserialize failures
        if self.deserialize_failures_total.load(Ordering::Relaxed) > 0 {
            violations.push(format!(
                "Deserialize failures detected: {}",
                self.deserialize_failures_total.load(Ordering::Relaxed)
            ));
        }

        violations
    }
}

// Alert system for critical events
pub struct AlertSystem {
    webhook_url: Option<String>,
}

impl AlertSystem {
    pub fn new(webhook_url: Option<String>) -> Self {
        Self { webhook_url }
    }

    pub async fn send_critical_alert(&self, message: &str) {
        log::error!("CRITICAL ALERT: {}", message);

        // Send to webhook if configured
        if let Some(url) = &self.webhook_url {
            let client = reqwest::Client::new();
            let payload = serde_json::json!({
                "text": format!("🚨 RACEBOARD CRITICAL: {}", message),
                "severity": "critical",
                "timestamp": Utc::now().to_rfc3339(),
            });

            if let Err(e) = client.post(url).json(&payload).send().await {
                log::error!("Failed to send alert to webhook: {}", e);
            }
        }

        // Also write to a dedicated alert log
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/raceboard_alerts.log")
        {
            use std::io::Write;
            let _ = writeln!(file, "{} - CRITICAL: {}", Utc::now().to_rfc3339(), message);
        }
    }

    pub async fn send_data_loss_alert(&self, race_id: &str, reason: &str) {
        let message = format!(
            "DATA LOSS: Race {} was deleted. Reason: {}. This impacts cluster rebuilding accuracy!",
            race_id, reason
        );
        self.send_critical_alert(&message).await;
    }
}
