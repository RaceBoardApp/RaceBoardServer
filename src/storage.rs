use crate::models::{Event, Race, RaceUpdate};
use crate::monitoring::MonitoringSystem;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

#[derive(Debug, Clone)]
pub enum StorageEvent {
    Created(Race),
    Updated(Race),
    Deleted(String),
}

#[derive(Debug)]
pub struct Storage {
    races: RwLock<HashMap<String, Race>>,
    event_sender: broadcast::Sender<StorageEvent>,
    max_races: usize,
    max_events_per_race: usize,
}

impl Storage {
    pub fn new() -> Self {
        // CRITICAL: Increased from 1000 to 100,000 to support cluster rebuilding
        // Cluster rebuilding needs extensive historic data
        Self::with_config(100_000, 1000)
    }

    pub fn with_config(max_races: usize, max_events_per_race: usize) -> Self {
        let (event_sender, _) = broadcast::channel(100);
        let races = HashMap::new();

        log::warn!(
            "Storage initialized with max_races={}, max_events_per_race={}",
            max_races,
            max_events_per_race
        );
        log::warn!(
            "CRITICAL: Ensure persistence layer is properly configured for cluster rebuilding"
        );

        Storage {
            races: RwLock::new(races),
            event_sender,
            max_races,
            max_events_per_race,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StorageEvent> {
        self.event_sender.subscribe()
    }

    pub async fn create_or_update_race(&self, mut race: Race) -> Race {
        let mut races = self.races.write().await;

        // Check if we need to make room
        if races.len() >= self.max_races && !races.contains_key(&race.id) {
            // CRITICAL: Log when we're at capacity - this affects cluster rebuilding!
            log::error!("STORAGE CAPACITY REACHED: {} races. Removing oldest race. This will impact cluster rebuilding!", races.len());

            // Find and remove the oldest race by started_at timestamp
            if let Some((oldest_id, _)) = races.iter().min_by_key(|(_, r)| r.started_at) {
                let oldest_id = oldest_id.clone();
                races.remove(&oldest_id);
                let _ = self
                    .event_sender
                    .send(StorageEvent::Deleted(oldest_id.clone()));
                log::warn!(
                    "Evicted race {} due to storage limit. Data loss occurred!",
                    oldest_id
                );
            }
        }

        // Limit events per race
        if let Some(ref mut events) = race.events {
            if events.len() > self.max_events_per_race {
                events.truncate(self.max_events_per_race);
            }
        }

        let is_update = races.contains_key(&race.id);

        // Generate new ID if not provided or empty
        if race.id.is_empty() {
            race.id = uuid::Uuid::new_v4().to_string();
        }

        races.insert(race.id.clone(), race.clone());

        // Send event
        let event = if is_update {
            StorageEvent::Updated(race.clone())
        } else {
            StorageEvent::Created(race.clone())
        };
        let _ = self.event_sender.send(event);

        race
    }

    pub async fn get_race(&self, id: &str) -> Option<Race> {
        let races = self.races.read().await;
        races.get(id).cloned()
    }

    pub async fn get_all_races(&self) -> Vec<Race> {
        let races = self.races.read().await;
        races.values().cloned().collect()
    }

    pub async fn update_race(&self, id: &str, update: RaceUpdate) -> Option<Race> {
        let mut races = self.races.write().await;

        if let Some(race) = races.get_mut(id) {
            race.apply_update(update);
            let updated = race.clone();
            let _ = self
                .event_sender
                .send(StorageEvent::Updated(updated.clone()));

            Some(updated)
        } else {
            None
        }
    }

    pub async fn add_event_to_race(&self, id: &str, event: Event) -> Option<Race> {
        let mut races = self.races.write().await;

        if let Some(race) = races.get_mut(id) {
            // Limit events
            if let Some(ref events) = race.events {
                if events.len() >= self.max_events_per_race {
                    return Some(race.clone());
                }
            }

            race.add_event(event);
            let updated = race.clone();
            let _ = self
                .event_sender
                .send(StorageEvent::Updated(updated.clone()));

            Some(updated)
        } else {
            None
        }
    }

    pub async fn delete_race(&self, id: &str) -> bool {
        let mut races = self.races.write().await;
        if races.remove(id).is_some() {
            let _ = self
                .event_sender
                .send(StorageEvent::Deleted(id.to_string()));

            true
        } else {
            false
        }
    }

    pub async fn clear_all(&self) {
        let mut races = self.races.write().await;
        races.clear();
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}
