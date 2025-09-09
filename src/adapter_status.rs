use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;

// ============================================================================
// Adapter Health State Machine
// ============================================================================

/// Strict adapter health states following the state machine model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdapterHealthState {
    /// Adapter has registered but not yet sent first health report
    /// Must transition to HEALTHY within 1.5×interval or become UNHEALTHY
    Registered,
    
    /// Adapter is reporting health within expected interval (≤1.5×interval)
    Healthy,
    
    /// Adapter missed recent reports (>1.5× but ≤3×interval)
    /// Can recover to HEALTHY if reports resume
    Unhealthy,
    
    /// No reports for extended period (>3×interval)
    /// Adapter process likely crashed or network issue
    Unknown,
    
    /// Adapter explicitly deregistered via API
    /// Clean shutdown state
    Stopped,
    
    /// Adapter doesn't require health monitoring (Claude, CMD)
    /// Always considered available
    Exempt,
}

impl AdapterHealthState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Registered => "REGISTERED",
            Self::Healthy => "HEALTHY", 
            Self::Unhealthy => "UNHEALTHY",
            Self::Unknown => "UNKNOWN",
            Self::Stopped => "STOPPED",
            Self::Exempt => "EXEMPT",
        }
    }
    
    /// Returns the color for UI display
    pub fn color(&self) -> &'static str {
        match self {
            Self::Registered => "blue",
            Self::Healthy => "green",
            Self::Unhealthy => "yellow",
            Self::Unknown => "orange",
            Self::Stopped => "gray",
            Self::Exempt => "purple",
        }
    }
    
    /// Returns severity level for alerting (0=info, 1=warning, 2=error, 3=critical)
    pub fn severity(&self) -> u8 {
        match self {
            Self::Registered => 0,
            Self::Healthy => 0,
            Self::Unhealthy => 1,
            Self::Unknown => 2,
            Self::Stopped => 0,
            Self::Exempt => 0,
        }
    }
}

/// Adapter types in the system
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdapterType {
    GitLab,
    Calendar,
    CodexWatch,
    GeminiWatch,
    Claude,
    Cmd,
}

impl AdapterType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitLab => "gitlab",
            Self::Calendar => "calendar",
            Self::CodexWatch => "codex-watch",
            Self::GeminiWatch => "gemini-watch",
            Self::Claude => "claude",
            Self::Cmd => "cmd",
        }
    }
    
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::GitLab => "GitLab CI",
            Self::Calendar => "Google Calendar", 
            Self::CodexWatch => "Codex Watch",
            Self::GeminiWatch => "Gemini Watch",
            Self::Claude => "Claude AI",
            Self::Cmd => "Command Runner",
        }
    }
    
    /// Returns true if this adapter type requires health monitoring
    pub fn requires_health_check(&self) -> bool {
        !matches!(self, Self::Claude | Self::Cmd)
    }
    
    /// Default health check interval in seconds
    pub fn default_interval_seconds(&self) -> u32 {
        match self {
            Self::GitLab => 30,
            Self::Calendar => 60,
            Self::CodexWatch => 30,
            Self::GeminiWatch => 30,
            Self::Claude | Self::Cmd => 0, // No health checks
        }
    }
}

/// Adapter registration with instance support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterRegistration {
    /// Unique adapter ID: "adapter:{type}:{instance}"
    pub id: String,
    
    /// Adapter type
    pub adapter_type: AdapterType,
    
    /// Instance identifier (e.g., hostname, UUID)
    pub instance_id: String,
    
    /// Display name for UI
    pub display_name: String,
    
    /// Adapter version
    pub version: String,
    
    /// Registration timestamp
    pub registered_at: DateTime<Utc>,
    
    /// Health check interval in seconds (0 = no health checks)
    pub health_interval_seconds: u32,
    
    /// Process ID if available
    pub pid: Option<u32>,
    
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl AdapterRegistration {
    /// Create adapter race ID
    pub fn race_id(&self) -> String {
        format!("adapter:{}:{}", self.adapter_type.as_str(), self.instance_id)
    }
}

/// Adapter health information with state machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterHealth {
    /// Current state in the state machine
    pub state: AdapterHealthState,
    
    /// Last health report received
    pub last_report: Option<DateTime<Utc>>,
    
    /// Expected interval for health reports
    pub expected_interval: Duration,
    
    /// Time when state last changed
    pub state_changed_at: DateTime<Utc>,
    
    /// Previous state before last transition
    pub previous_state: Option<AdapterHealthState>,
    
    /// Metrics from last health report
    pub metrics: AdapterMetrics,
    
    /// Error message if unhealthy
    pub error: Option<String>,
}

impl AdapterHealth {
    /// Create new health tracking for registered adapter
    pub fn new_registered(interval_seconds: u32) -> Self {
        Self {
            state: if interval_seconds == 0 {
                AdapterHealthState::Exempt
            } else {
                AdapterHealthState::Registered
            },
            last_report: None,
            expected_interval: Duration::seconds(interval_seconds as i64),
            state_changed_at: Utc::now(),
            previous_state: None,
            metrics: AdapterMetrics::default(),
            error: None,
        }
    }
    
    /// Calculate seconds since last report
    pub fn seconds_since_report(&self) -> Option<i64> {
        self.last_report.map(|t| (Utc::now() - t).num_seconds())
    }
    
    /// Check if health report is overdue based on thresholds
    pub fn check_thresholds(&self) -> AdapterHealthState {
        if self.state == AdapterHealthState::Exempt {
            return AdapterHealthState::Exempt;
        }
        
        if self.state == AdapterHealthState::Stopped {
            return AdapterHealthState::Stopped;
        }
        
        let Some(seconds_since) = self.seconds_since_report() else {
            // No report yet after registration
            if self.state == AdapterHealthState::Registered {
                let since_registration = (Utc::now() - self.state_changed_at).num_seconds();
                let threshold = (self.expected_interval.num_seconds() as f64 * 1.5) as i64;
                if since_registration > threshold {
                    return AdapterHealthState::Unhealthy;
                }
            }
            return self.state;
        };
        
        let interval = self.expected_interval.num_seconds();
        let healthy_threshold = (interval as f64 * 1.5) as i64;
        let unhealthy_threshold = (interval as f64 * 3.0) as i64;
        
        match seconds_since {
            s if s <= healthy_threshold => AdapterHealthState::Healthy,
            s if s <= unhealthy_threshold => AdapterHealthState::Unhealthy,
            _ => AdapterHealthState::Unknown,
        }
    }
    
    /// Process state transition
    pub fn transition_to(&mut self, new_state: AdapterHealthState) {
        if self.state != new_state {
            self.previous_state = Some(self.state);
            self.state = new_state;
            self.state_changed_at = Utc::now();
        }
    }
    
    /// Update with new health report
    pub fn update_report(&mut self, metrics: AdapterMetrics, error: Option<String>) {
        self.last_report = Some(Utc::now());
        self.metrics = metrics;
        
        // Check and update state based on new report
        let new_state = if error.is_some() {
            AdapterHealthState::Unhealthy
        } else {
            AdapterHealthState::Healthy
        };
        
        self.error = error;
        self.transition_to(new_state);
    }
}

/// Adapter metrics from health reports
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterMetrics {
    pub races_created: u64,
    pub races_updated: u64,
    pub last_activity: Option<DateTime<Utc>>,
    pub error_count: u64,
    pub response_time_ms: Option<u64>,
    pub memory_usage_bytes: Option<u64>,
    pub cpu_usage_percent: Option<f32>,
}

// ============================================================================
// Adapter Registry with State Machine
// ============================================================================

/// Thread-safe adapter registry
#[derive(Clone)]
pub struct AdapterRegistry {
    /// Map of adapter ID to registration + health
    adapters: Arc<RwLock<HashMap<String, (AdapterRegistration, AdapterHealth)>>>,
    
    /// Last registry update time
    last_update: Arc<RwLock<DateTime<Utc>>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
            last_update: Arc::new(RwLock::new(Utc::now())),
        }
    }
    
    /// Register a new adapter
    pub async fn register(&self, registration: AdapterRegistration) -> Result<()> {
        let mut adapters = self.adapters.write().await;
        
        let health = AdapterHealth::new_registered(registration.health_interval_seconds);
        adapters.insert(registration.id.clone(), (registration, health));
        
        *self.last_update.write().await = Utc::now();
        Ok(())
    }
    
    /// Process health report from adapter
    pub async fn report_health(
        &self, 
        adapter_id: &str,
        metrics: AdapterMetrics,
        error: Option<String>
    ) -> Result<()> {
        let mut adapters = self.adapters.write().await;
        
        if let Some((_, health)) = adapters.get_mut(adapter_id) {
            health.update_report(metrics, error);
        }
        
        *self.last_update.write().await = Utc::now();
        Ok(())
    }
    
    /// Deregister adapter (clean shutdown)
    pub async fn deregister(&self, adapter_id: &str) -> Result<()> {
        let mut adapters = self.adapters.write().await;
        
        if let Some((_, health)) = adapters.get_mut(adapter_id) {
            health.transition_to(AdapterHealthState::Stopped);
        }
        
        Ok(())
    }
    
    /// Check all adapter states and update based on thresholds
    pub async fn update_states(&self) {
        let mut adapters = self.adapters.write().await;
        
        for (_, health) in adapters.values_mut() {
            let new_state = health.check_thresholds();
            health.transition_to(new_state);
        }
        
        *self.last_update.write().await = Utc::now();
    }
    
    /// Get all adapters with current state
    pub async fn get_all(&self) -> Vec<(AdapterRegistration, AdapterHealth)> {
        let adapters = self.adapters.read().await;
        adapters.values().cloned().collect()
    }
    
    /// Get specific adapter
    pub async fn get(&self, adapter_id: &str) -> Option<(AdapterRegistration, AdapterHealth)> {
        let adapters = self.adapters.read().await;
        adapters.get(adapter_id).cloned()
    }
    
    /// Get system summary
    pub async fn get_summary(&self) -> SystemSummary {
        let adapters = self.adapters.read().await;
        
        let mut state_counts = HashMap::new();
        let mut total_races_created = 0u64;
        let mut total_races_updated = 0u64;
        
        for (_, health) in adapters.values() {
            *state_counts.entry(health.state).or_insert(0) += 1;
            total_races_created += health.metrics.races_created;
            total_races_updated += health.metrics.races_updated;
        }
        
        SystemSummary {
            total_adapters: adapters.len(),
            state_counts,
            total_races_created,
            total_races_updated,
            last_update: *self.last_update.read().await,
        }
    }
    
    /// Remove stale registrations (not reported for >1 hour)
    pub async fn cleanup_stale(&self) {
        let mut adapters = self.adapters.write().await;
        let one_hour_ago = Utc::now() - Duration::hours(1);
        
        adapters.retain(|_, (reg, health)| {
            // Keep exempt and stopped adapters
            if matches!(health.state, AdapterHealthState::Exempt | AdapterHealthState::Stopped) {
                return true;
            }
            
            // Remove if in UNKNOWN state for >1 hour
            if health.state == AdapterHealthState::Unknown {
                return health.state_changed_at > one_hour_ago;
            }
            
            true
        });
    }
}

/// System-wide summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSummary {
    pub total_adapters: usize,
    pub state_counts: HashMap<AdapterHealthState, usize>,
    pub total_races_created: u64,
    pub total_races_updated: u64,
    pub last_update: DateTime<Utc>,
}

impl SystemSummary {
    pub fn healthy_count(&self) -> usize {
        self.state_counts.get(&AdapterHealthState::Healthy).copied().unwrap_or(0)
    }
    
    pub fn unhealthy_count(&self) -> usize {
        self.state_counts.get(&AdapterHealthState::Unhealthy).copied().unwrap_or(0)
    }
    
    pub fn all_operational(&self) -> bool {
        self.unhealthy_count() == 0 && 
        self.state_counts.get(&AdapterHealthState::Unknown).copied().unwrap_or(0) == 0
    }
}

// ============================================================================
// Monitoring Background Job
// ============================================================================

/// Background job that monitors adapter health states
pub struct AdapterMonitor {
    registry: AdapterRegistry,
    check_interval: Duration,
}

impl AdapterMonitor {
    pub fn new(registry: AdapterRegistry) -> Self {
        Self {
            registry,
            check_interval: Duration::seconds(5), // Check every 5 seconds
        }
    }
    
    /// Run monitoring loop
    pub async fn run(self) {
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.check_interval.num_seconds() as u64)
        );
        
        loop {
            interval.tick().await;
            
            // Update all adapter states based on timing thresholds
            self.registry.update_states().await;
            
            // Check for alerts
            self.check_alerts().await;
            
            // Cleanup stale registrations periodically (every 12 iterations = 1 minute)
            static CLEANUP_COUNTER: std::sync::atomic::AtomicU32 = 
                std::sync::atomic::AtomicU32::new(0);
            
            if CLEANUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 12 == 0 {
                self.registry.cleanup_stale().await;
            }
        }
    }
    
    /// Check for alert conditions
    async fn check_alerts(&self) {
        let adapters = self.registry.get_all().await;
        
        for (reg, health) in adapters {
            // Alert if critical adapter goes unhealthy
            if matches!(reg.adapter_type, AdapterType::GitLab | AdapterType::Calendar) {
                if health.state == AdapterHealthState::Unknown {
                    tracing::error!(
                        adapter_id = %reg.id,
                        adapter_type = ?reg.adapter_type,
                        "Critical adapter is in UNKNOWN state"
                    );
                }
            }
            
            // Log state transitions
            if let Some(prev) = health.previous_state {
                if prev != health.state {
                    tracing::info!(
                        adapter_id = %reg.id,
                        from = %prev.as_str(),
                        to = %health.state.as_str(),
                        "Adapter state transition"
                    );
                }
            }
        }
    }
}

// ============================================================================
// Prometheus Metrics Export
// ============================================================================

impl AdapterRegistry {
    /// Export metrics in Prometheus format
    pub async fn export_metrics(&self) -> String {
        let mut output = String::new();
        let adapters = self.get_all().await;
        let summary = self.get_summary().await;
        
        // System-wide metrics
        output.push_str(&format!(
            "# HELP raceboard_adapters_total Total number of registered adapters\n\
             # TYPE raceboard_adapters_total gauge\n\
             raceboard_adapters_total {}\n\n",
            summary.total_adapters
        ));
        
        // State counts
        for (state, count) in summary.state_counts {
            output.push_str(&format!(
                "raceboard_adapters_state{{state=\"{}\"}} {}\n",
                state.as_str(),
                count
            ));
        }
        output.push('\n');
        
        // Per-adapter metrics
        for (reg, health) in adapters {
            let labels = format!(
                "adapter_type=\"{}\",instance=\"{}\",state=\"{}\"",
                reg.adapter_type.as_str(),
                reg.instance_id,
                health.state.as_str()
            );
            
            // Health state as enum value
            output.push_str(&format!(
                "raceboard_adapter_health{{{}}} {}\n",
                labels,
                health.state.severity()
            ));
            
            // Seconds since last report
            if let Some(seconds) = health.seconds_since_report() {
                output.push_str(&format!(
                    "raceboard_adapter_last_report_seconds{{{}}} {}\n",
                    labels,
                    seconds
                ));
            }
            
            // Races created/updated
            output.push_str(&format!(
                "raceboard_adapter_races_created{{{}}} {}\n",
                labels,
                health.metrics.races_created
            ));
            
            output.push_str(&format!(
                "raceboard_adapter_races_updated{{{}}} {}\n",
                labels,
                health.metrics.races_updated
            ));
        }
        
        output
    }
}