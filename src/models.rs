use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// For ETA history tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtaRevision {
    pub eta_sec: i64,
    pub timestamp: DateTime<Utc>,
    pub source: i32, // Maps to proto EtaSource
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaceState {
    Queued,
    Running,
    Passed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Race {
    pub id: String,
    pub source: String,
    pub title: String,
    pub state: RaceState,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<i64>, // Calculated by server when race completes
    pub eta_sec: Option<i64>,
    pub progress: Option<i32>,
    pub deeplink: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<Event>>,
    
    // New fields for optimistic progress support
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_progress_update: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_eta_update: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_source: Option<i32>, // Maps to proto EtaSource enum
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_interval_hint: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_history: Option<Vec<EtaRevision>>,
}

impl Race {
    pub fn new(source: String, title: String) -> Self {
        Race {
            id: Uuid::new_v4().to_string(),
            source,
            title,
            state: RaceState::Queued,
            started_at: Utc::now(),
            completed_at: None,
            duration_sec: None,
            eta_sec: None,
            progress: None,
            deeplink: None,
            metadata: None,
            events: Some(Vec::new()),
            // Initialize new fields
            last_progress_update: None,
            last_eta_update: None,
            eta_source: None,
            eta_confidence: None,
            update_interval_hint: None,
            eta_history: None,
        }
    }

    pub fn apply_update(&mut self, update: RaceUpdate) {
        if let Some(source) = update.source {
            self.source = source;
        }
        if let Some(title) = update.title {
            self.title = title;
        }
        if let Some(state) = update.state {
            // Calculate duration when race completes
            let was_running = matches!(self.state, RaceState::Running | RaceState::Queued);
            let is_completed = matches!(
                state,
                RaceState::Passed | RaceState::Failed | RaceState::Canceled
            );

            if was_running && is_completed && self.completed_at.is_none() {
                let now = Utc::now();
                self.completed_at = Some(now);
                self.duration_sec = Some((now - self.started_at).num_seconds());
            }

            self.state = state;
        }
        if let Some(started_at) = update.started_at {
            self.started_at = started_at;
        }
        if let Some(eta_sec) = update.eta_sec {
            // Track ETA changes for history
            if self.eta_sec != Some(eta_sec) {
                self.last_eta_update = Some(Utc::now());
                
                // Add to history
                let revision = EtaRevision {
                    eta_sec,
                    timestamp: Utc::now(),
                    source: self.eta_source.unwrap_or(0), // Will be inferred later
                    confidence: self.eta_confidence,
                };
                
                if let Some(ref mut history) = self.eta_history {
                    history.push(revision);
                    // Keep only last 5
                    if history.len() > 5 {
                        history.remove(0);
                    }
                } else {
                    self.eta_history = Some(vec![revision]);
                }
            }
            self.eta_sec = Some(eta_sec);
        }
        if let Some(progress) = update.progress {
            // Track progress changes
            if self.progress != Some(progress) {
                self.last_progress_update = Some(Utc::now());
            }
            self.progress = Some(progress);
        }
        if let Some(deeplink) = update.deeplink {
            self.deeplink = Some(deeplink);
        }
        if let Some(metadata) = update.metadata {
            self.metadata = Some(metadata);
        }
        // Handle new optimistic progress fields
        if let Some(eta_source) = update.eta_source {
            self.eta_source = Some(eta_source);
        }
        if let Some(eta_confidence) = update.eta_confidence {
            self.eta_confidence = Some(eta_confidence);
        }
        if let Some(update_interval_hint) = update.update_interval_hint {
            self.update_interval_hint = Some(update_interval_hint);
        }
    }
    
    // Infer ETA source based on source name if not set
    pub fn infer_eta_source(&mut self) {
        if self.eta_source.is_none() && self.eta_sec.is_some() {
            self.eta_source = Some(match self.source.as_str() {
                "google-calendar" => 1, // EtaSource::Exact
                "gitlab" | "github" | "jenkins" => 2, // EtaSource::Adapter
                _ => 2, // Default to Adapter
            });
        }
    }
    
    // Infer confidence based on source
    pub fn infer_eta_confidence(&mut self) {
        if self.eta_confidence.is_none() && self.eta_source.is_some() {
            self.eta_confidence = Some(match self.eta_source.unwrap() {
                1 => 1.0,   // Exact
                3 => 0.7,   // Cluster
                2 => 0.5,   // Adapter
                4 => 0.2,   // Bootstrap
                _ => 0.3,
            });
        }
    }

    // Infer update interval hint based on source
    pub fn infer_update_interval_hint(&mut self) {
        if self.update_interval_hint.is_none() && self.eta_source.is_some() {
            self.update_interval_hint = Some(match self.eta_source.unwrap() {
                1 => 60,    // Exact: 60s
                2 => 10,    // Adapter: 10s
                3 => 15,    // Cluster: 15s
                4 => 10,    // Bootstrap: 10s
                _ => 10,
            });
        }
    }

    pub fn add_event(&mut self, event: Event) {
        if let Some(ref mut events) = self.events {
            events.push(event);
        } else {
            self.events = Some(vec![event]);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaceUpdate {
    pub source: Option<String>,
    pub title: Option<String>,
    pub state: Option<RaceState>,
    pub started_at: Option<DateTime<Utc>>,
    pub eta_sec: Option<i64>,
    pub progress: Option<i32>,
    pub deeplink: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    // New optional fields from adapters
    pub eta_source: Option<i32>,
    pub eta_confidence: Option<f64>,
    pub update_interval_hint: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: Option<serde_json::Value>,
    #[serde(default = "Utc::now")]
    pub timestamp: DateTime<Utc>,
}

impl Event {
    pub fn new(event_type: String, data: Option<serde_json::Value>) -> Self {
        Event {
            event_type,
            data,
            timestamp: Utc::now(),
        }
    }
}
