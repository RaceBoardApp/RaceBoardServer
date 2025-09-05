use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::hnsw_dbscan::{ValidationCriteria, ValidationResult};
use crate::rebuild::SourceConfig;

/// Phased rollout controller for gradual cluster rebuilding deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhasedRollout {
    pub current_phase: RolloutPhase,
    pub phase_history: Vec<PhaseTransition>,
    pub source_status: HashMap<String, SourceRolloutStatus>,
    pub global_metrics: RolloutMetrics,
    pub config: RolloutConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RolloutPhase {
    /// Phase 1: Single source (e.g., "cargo")
    SingleSource,
    /// Phase 2: All sources with conservative parameters
    AllSourcesConservative,
    /// Phase 3: Automatic parameter tuning enabled
    AutomaticTuning,
    /// Rollback phase if issues detected
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseTransition {
    pub from_phase: RolloutPhase,
    pub to_phase: RolloutPhase,
    pub timestamp: DateTime<Utc>,
    pub reason: String,
    pub metrics_snapshot: RolloutMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRolloutStatus {
    pub source: String,
    pub enabled: bool,
    pub mode: RolloutMode,
    pub last_rebuild: Option<DateTime<Utc>>,
    pub success_count: u32,
    pub failure_count: u32,
    pub current_parameters: SourceConfig,
    pub validation_results: Vec<ValidationResult>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RolloutMode {
    /// Not yet enabled for rebuilding
    Disabled,
    /// Running in shadow mode (rebuild but don't use results)
    Shadow,
    /// Running as canary (use for small % of traffic)
    Canary { percentage: u8 },
    /// Fully enabled
    Production,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutMetrics {
    pub total_rebuilds: u64,
    pub successful_rebuilds: u64,
    pub failed_rebuilds: u64,
    pub average_mae: f64,
    pub average_noise_ratio: f64,
    pub average_ari: f64,
    pub rollback_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutConfig {
    pub pilot_source: String, // Source to use for Phase 1
    pub shadow_duration: Duration,
    pub canary_duration: Duration,
    pub canary_percentage: u8,
    pub success_threshold: f64, // e.g., 0.95 for 95% success rate
    pub min_rebuilds_for_promotion: u32,
    pub auto_rollback: bool,
    pub validation_criteria: ValidationCriteria,
}

impl Default for RolloutConfig {
    fn default() -> Self {
        Self {
            pilot_source: "ci".to_string(),
            shadow_duration: Duration::days(1),
            canary_duration: Duration::days(2),
            canary_percentage: 10,
            success_threshold: 0.95,
            min_rebuilds_for_promotion: 10,
            auto_rollback: true,
            validation_criteria: ValidationCriteria::default(),
        }
    }
}

impl PhasedRollout {
    pub fn new(config: RolloutConfig) -> Self {
        // Start with empty source_status - will be populated dynamically
        Self {
            current_phase: RolloutPhase::SingleSource,
            phase_history: Vec::new(),
            source_status: HashMap::new(),
            global_metrics: RolloutMetrics {
                total_rebuilds: 0,
                successful_rebuilds: 0,
                failed_rebuilds: 0,
                average_mae: 0.0,
                average_noise_ratio: 0.0,
                average_ari: 0.0,
                rollback_count: 0,
            },
            config,
        }
    }

    /// Register a new source if it doesn't exist
    pub fn register_source(&mut self, source: &str) {
        if !self.source_status.contains_key(source) {
            log::info!("Registering new source in rollout: {}", source);
            self.source_status.insert(
                source.to_string(),
                SourceRolloutStatus {
                    source: source.to_string(),
                    enabled: false,
                    mode: RolloutMode::Disabled,
                    last_rebuild: None,
                    success_count: 0,
                    failure_count: 0,
                    current_parameters: SourceConfig {
                        eps_range: (0.3, 0.5),
                        min_samples: 3,
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
                    validation_results: Vec::new(),
                },
            );
        }
    }

    /// Register multiple sources at once
    pub fn register_sources(&mut self, sources: &[String]) {
        for source in sources {
            self.register_source(source);
        }
    }

    /// Enable all known sources with a given rollout mode
    pub fn enable_all_sources(&mut self, mode: RolloutMode) {
        for (_source, status) in self.source_status.iter_mut() {
            status.enabled = true;
            status.mode = mode;
            status.last_rebuild = status.last_rebuild.or(Some(Utc::now()));
        }
    }

    /// Start Phase 1: Enable single pilot source
    pub fn start_phase_1(&mut self) -> Result<()> {
        if let Some(status) = self.source_status.get_mut(&self.config.pilot_source) {
            status.enabled = true;
            status.mode = RolloutMode::Shadow;

            self.add_transition(
                RolloutPhase::SingleSource,
                RolloutPhase::SingleSource,
                format!("Started Phase 1 with source: {}", self.config.pilot_source),
            );

            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Pilot source not found: {}",
                self.config.pilot_source
            ))
        }
    }

    /// Promote source from shadow to canary mode
    pub fn promote_to_canary(&mut self, source: &str) -> Result<()> {
        if let Some(status) = self.source_status.get_mut(source) {
            if status.mode != RolloutMode::Shadow {
                return Err(anyhow::anyhow!(
                    "Source must be in shadow mode to promote to canary"
                ));
            }

            let success_rate = status.success_count as f64
                / (status.success_count + status.failure_count).max(1) as f64;

            if success_rate < self.config.success_threshold {
                return Err(anyhow::anyhow!(
                    "Success rate {:.2}% below threshold {:.2}%",
                    success_rate * 100.0,
                    self.config.success_threshold * 100.0
                ));
            }

            status.mode = RolloutMode::Canary {
                percentage: self.config.canary_percentage,
            };

            Ok(())
        } else {
            Err(anyhow::anyhow!("Source not found: {}", source))
        }
    }

    /// Promote source from canary to production
    pub fn promote_to_production(&mut self, source: &str) -> Result<()> {
        if let Some(status) = self.source_status.get_mut(source) {
            if !matches!(status.mode, RolloutMode::Canary { .. }) {
                return Err(anyhow::anyhow!(
                    "Source must be in canary mode to promote to production"
                ));
            }

            status.mode = RolloutMode::Production;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Source not found: {}", source))
        }
    }

    /// Advance to next phase if criteria met
    pub fn try_advance_phase(&mut self) -> Result<bool> {
        match self.current_phase {
            RolloutPhase::SingleSource => {
                // Check if pilot source is ready for production
                if let Some(status) = self.source_status.get(&self.config.pilot_source) {
                    if status.mode == RolloutMode::Production
                        && status.success_count >= self.config.min_rebuilds_for_promotion
                    {
                        // Move to Phase 2: Enable all sources conservatively
                        self.current_phase = RolloutPhase::AllSourcesConservative;

                        for (source, status) in self.source_status.iter_mut() {
                            if source != &self.config.pilot_source {
                                status.enabled = true;
                                status.mode = RolloutMode::Shadow;
                            }
                        }

                        self.add_transition(
                            RolloutPhase::SingleSource,
                            RolloutPhase::AllSourcesConservative,
                            "Phase 1 successful, enabling all sources".to_string(),
                        );

                        return Ok(true);
                    }
                }
            }

            RolloutPhase::AllSourcesConservative => {
                // Check if all sources are in production
                let all_production = self
                    .source_status
                    .values()
                    .all(|s| s.mode == RolloutMode::Production);

                let total_success = self
                    .source_status
                    .values()
                    .map(|s| s.success_count)
                    .sum::<u32>();

                if all_production && total_success >= self.config.min_rebuilds_for_promotion * 5 {
                    // Move to Phase 3: Enable automatic tuning
                    self.current_phase = RolloutPhase::AutomaticTuning;

                    self.add_transition(
                        RolloutPhase::AllSourcesConservative,
                        RolloutPhase::AutomaticTuning,
                        "Phase 2 successful, enabling automatic parameter tuning".to_string(),
                    );

                    return Ok(true);
                }
            }

            RolloutPhase::AutomaticTuning => {
                // Final phase - no advancement
                return Ok(false);
            }

            RolloutPhase::Rollback => {
                // In rollback - manual intervention needed
                return Ok(false);
            }
        }

        Ok(false)
    }

    /// Record rebuild result and update metrics
    pub fn record_rebuild_result(&mut self, source: &str, result: ValidationResult) {
        self.global_metrics.total_rebuilds += 1;

        if result.passed {
            self.global_metrics.successful_rebuilds += 1;
            if let Some(status) = self.source_status.get_mut(source) {
                status.success_count += 1;
                status.last_rebuild = Some(Utc::now());
                status.validation_results.push(result.clone());

                // Keep only last 10 results
                if status.validation_results.len() > 10 {
                    status.validation_results.remove(0);
                }
            }
        } else {
            self.global_metrics.failed_rebuilds += 1;
            if let Some(status) = self.source_status.get_mut(source) {
                status.failure_count += 1;
                status.validation_results.push(result.clone());
            }

            // Check for rollback conditions
            if self.config.auto_rollback {
                self.check_rollback_conditions();
            }
        }

        // Update rolling averages
        self.update_global_metrics(&result);
    }

    /// Check if rollback is needed
    fn check_rollback_conditions(&mut self) {
        let recent_failures = self
            .source_status
            .values()
            .map(|s| {
                let recent = s
                    .validation_results
                    .iter()
                    .rev()
                    .take(5)
                    .filter(|r| !r.passed)
                    .count();
                recent as f64 / 5.0
            })
            .sum::<f64>()
            / self.source_status.len() as f64;

        if recent_failures > 0.5 {
            // More than 50% failures recently
            self.trigger_rollback("High failure rate detected");
        }
    }

    /// Trigger emergency rollback
    pub fn trigger_rollback(&mut self, reason: &str) {
        let prev_phase = self.current_phase;
        self.current_phase = RolloutPhase::Rollback;
        self.global_metrics.rollback_count += 1;

        // Disable all sources
        for status in self.source_status.values_mut() {
            status.enabled = false;
            status.mode = RolloutMode::Disabled;
        }

        self.add_transition(prev_phase, RolloutPhase::Rollback, reason.to_string());
    }

    /// Add phase transition to history
    fn add_transition(&mut self, from: RolloutPhase, to: RolloutPhase, reason: String) {
        self.phase_history.push(PhaseTransition {
            from_phase: from,
            to_phase: to,
            timestamp: Utc::now(),
            reason,
            metrics_snapshot: self.global_metrics.clone(),
        });
    }

    /// Update global metrics with new result
    fn update_global_metrics(&mut self, result: &ValidationResult) {
        let alpha = 0.1; // EMA smoothing factor

        self.global_metrics.average_mae =
            alpha * result.metrics.mae + (1.0 - alpha) * self.global_metrics.average_mae;

        self.global_metrics.average_noise_ratio = alpha * result.metrics.noise_ratio
            + (1.0 - alpha) * self.global_metrics.average_noise_ratio;

        self.global_metrics.average_ari =
            alpha * result.metrics.ari + (1.0 - alpha) * self.global_metrics.average_ari;
    }

    /// Check if a source should be used for a given request
    pub fn should_use_source(&self, source: &str, request_hash: u64) -> bool {
        if let Some(status) = self.source_status.get(source) {
            // First check if the source is enabled at all
            if !status.enabled {
                return false;
            }

            match status.mode {
                RolloutMode::Disabled => false,
                RolloutMode::Shadow => true, // We DO rebuild in shadow mode, just don't use results
                RolloutMode::Production => true,
                RolloutMode::Canary { percentage } => {
                    // Use hash to deterministically assign requests
                    (request_hash % 100) < percentage as u64
                }
            }
        } else {
            false
        }
    }
}
