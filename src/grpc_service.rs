use crate::adapter_status::AdapterRegistry;
use crate::persistence::PersistenceLayer;
use crate::storage::{Storage, StorageEvent};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub mod raceboard {
    tonic::include_proto!("raceboard");
}

use raceboard::race_service_server::RaceService;
use raceboard::{
    race_update, AddEventRequest, DeleteRaceRequest, Event as ProtoEvent, GetRaceRequest,
    Race as ProtoRace, RaceList, RaceState as ProtoRaceState, RaceUpdate, UpdateRaceRequest,
    SystemStatus, AdapterStatus as ProtoAdapterStatus, AdapterHealthState as ProtoHealthState,
    AdapterType as ProtoAdapterType, AdapterMetrics as ProtoMetrics,
};

pub struct RaceServiceImpl {
    storage: Arc<Storage>,
    persistence: Arc<PersistenceLayer>,
    adapter_registry: Arc<AdapterRegistry>,
}

impl RaceServiceImpl {
    pub fn new(
        storage: Arc<Storage>, 
        persistence: Arc<PersistenceLayer>,
        adapter_registry: Arc<AdapterRegistry>,
    ) -> Self {
        Self {
            storage,
            persistence,
            adapter_registry,
        }
    }
}

// Conversion helpers
fn convert_to_proto_race(race: crate::models::Race) -> ProtoRace {
    // Infer ETA source if not already set
    let eta_source = race.eta_source.or_else(|| {
        if race.eta_sec.is_some() {
            Some(match race.source.as_str() {
                "google-calendar" => raceboard::EtaSource::Exact as i32,
                "gitlab" | "github" | "jenkins" => raceboard::EtaSource::Adapter as i32,
                _ => raceboard::EtaSource::Adapter as i32,
            })
        } else {
            None
        }
    });
    
    // Infer confidence if not set
    let eta_confidence = race.eta_confidence.or_else(|| {
        eta_source.and_then(|source| {
            raceboard::EtaSource::try_from(source).ok().map(|s| match s {
                raceboard::EtaSource::Exact => 1.0,
                raceboard::EtaSource::Cluster => 0.7,
                raceboard::EtaSource::Adapter => 0.5,
                raceboard::EtaSource::Bootstrap => 0.2,
                _ => 0.3,
            })
        })
    });
    
    // Infer update_interval_hint if not set
    let update_interval_hint = race.update_interval_hint.or_else(|| {
        eta_source.and_then(|source| {
            raceboard::EtaSource::try_from(source).ok().map(|s| match s {
                raceboard::EtaSource::Exact => 60,
                raceboard::EtaSource::Adapter => 10,
                raceboard::EtaSource::Cluster => 15,
                raceboard::EtaSource::Bootstrap => 10,
                _ => 10,
            })
        })
    });
    
    ProtoRace {
        id: race.id,
        source: race.source,
        title: race.title,
        state: match race.state {
            crate::models::RaceState::Queued => ProtoRaceState::Queued as i32,
            crate::models::RaceState::Running => ProtoRaceState::Running as i32,
            crate::models::RaceState::Passed => ProtoRaceState::Passed as i32,
            crate::models::RaceState::Failed => ProtoRaceState::Failed as i32,
            crate::models::RaceState::Canceled => ProtoRaceState::Canceled as i32,
        },
        started_at: Some(prost_types::Timestamp {
            seconds: race.started_at.timestamp(),
            nanos: race.started_at.timestamp_subsec_nanos() as i32,
        }),
        eta_sec: race.eta_sec,
        progress: race.progress,
        deeplink: race.deeplink,
        metadata: race.metadata.unwrap_or_default(),
        events: race
            .events
            .unwrap_or_default()
            .into_iter()
            .map(|e| ProtoEvent {
                r#type: e.event_type,
                data: e.data.map(|d| d.to_string()),
                timestamp: Some(prost_types::Timestamp {
                    seconds: e.timestamp.timestamp(),
                    nanos: e.timestamp.timestamp_subsec_nanos() as i32,
                }),
            })
            .collect(),
        // New optional fields for optimistic progress
        last_progress_update: race.last_progress_update.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        }),
        last_eta_update: race.last_eta_update.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        }),
        eta_source,
        eta_confidence,
        update_interval_hint,
        eta_history: race.eta_history.unwrap_or_default().into_iter().map(|rev| {
            raceboard::EtaRevision {
                eta_sec: rev.eta_sec,
                timestamp: Some(prost_types::Timestamp {
                    seconds: rev.timestamp.timestamp(),
                    nanos: rev.timestamp.timestamp_subsec_nanos() as i32,
                }),
                source: rev.source,
                confidence: rev.confidence,
            }
        }).collect(),
    }
}

fn convert_from_proto_race(proto: ProtoRace) -> crate::models::Race {
    use chrono::{DateTime, Utc};

    crate::models::Race {
        id: proto.id,
        source: proto.source,
        title: proto.title,
        state: match ProtoRaceState::try_from(proto.state) {
            Ok(ProtoRaceState::Queued) => crate::models::RaceState::Queued,
            Ok(ProtoRaceState::Running) => crate::models::RaceState::Running,
            Ok(ProtoRaceState::Passed) => crate::models::RaceState::Passed,
            Ok(ProtoRaceState::Failed) => crate::models::RaceState::Failed,
            Ok(ProtoRaceState::Canceled) => crate::models::RaceState::Canceled,
            _ => crate::models::RaceState::Queued,
        },
        started_at: proto
            .started_at
            .and_then(|ts| DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
            .unwrap_or_else(Utc::now),
        completed_at: None, // Will be set when race completes
        duration_sec: None, // Will be calculated when race completes
        eta_sec: proto.eta_sec,
        progress: proto.progress,
        deeplink: proto.deeplink,
        metadata: if proto.metadata.is_empty() {
            None
        } else {
            Some(proto.metadata)
        },
        events: if proto.events.is_empty() {
            None
        } else {
            Some(
                proto
                    .events
                    .into_iter()
                    .map(|e| crate::models::Event {
                        event_type: e.r#type,
                        data: e.data.and_then(|d| serde_json::from_str(&d).ok()),
                        timestamp: e
                            .timestamp
                            .and_then(|ts| DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
                            .unwrap_or_else(Utc::now),
                    })
                    .collect(),
            )
        },
        // New optimistic progress fields
        last_progress_update: proto.last_progress_update
            .and_then(|ts| DateTime::from_timestamp(ts.seconds, ts.nanos as u32)),
        last_eta_update: proto.last_eta_update
            .and_then(|ts| DateTime::from_timestamp(ts.seconds, ts.nanos as u32)),
        eta_source: proto.eta_source,
        eta_confidence: proto.eta_confidence,
        update_interval_hint: proto.update_interval_hint,
        eta_history: if proto.eta_history.is_empty() {
            None
        } else {
            Some(
                proto.eta_history.into_iter().map(|rev| crate::models::EtaRevision {
                    eta_sec: rev.eta_sec,
                    timestamp: rev.timestamp
                        .and_then(|ts| DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
                        .unwrap_or_else(Utc::now),
                    source: rev.source,
                    confidence: rev.confidence,
                }).collect()
            )
        },
    }
}

#[tonic::async_trait]
impl RaceService for RaceServiceImpl {
    type StreamRacesStream = tokio_stream::wrappers::ReceiverStream<Result<RaceUpdate, Status>>;

    async fn stream_races(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::StreamRacesStream>, Status> {
        let mut receiver = self.storage.subscribe();

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Send initial snapshot
        let races = self.storage.get_all_races().await;
        for race in races {
            let update = RaceUpdate {
                r#type: race_update::UpdateType::Created as i32,
                race: Some(convert_to_proto_race(race)),
            };
            let _ = tx.send(Ok(update)).await;
        }

        // Stream updates
        tokio::spawn(async move {
            while let Ok(event) = receiver.recv().await {
                let update = match event {
                    StorageEvent::Created(race) => RaceUpdate {
                        r#type: race_update::UpdateType::Created as i32,
                        race: Some(convert_to_proto_race(race)),
                    },
                    StorageEvent::Updated(race) => RaceUpdate {
                        r#type: race_update::UpdateType::Updated as i32,
                        race: Some(convert_to_proto_race(race)),
                    },
                    StorageEvent::Deleted(id) => RaceUpdate {
                        r#type: race_update::UpdateType::Deleted as i32,
                        race: Some(ProtoRace {
                            id,
                            ..Default::default()
                        }),
                    },
                };

                if tx.send(Ok(update)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn get_race(
        &self,
        request: Request<GetRaceRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        let id = request.into_inner().id;

        match self.storage.get_race(&id).await {
            Some(race) => Ok(Response::new(convert_to_proto_race(race))),
            None => Err(Status::not_found(format!("Race {} not found", id))),
        }
    }

    async fn create_race(
        &self,
        request: Request<ProtoRace>,
    ) -> Result<Response<ProtoRace>, Status> {
        let proto_race = request.into_inner();
        let race = convert_from_proto_race(proto_race);
        let created = self.storage.create_or_update_race(race).await;
        Ok(Response::new(convert_to_proto_race(created)))
    }

    async fn update_race(
        &self,
        request: Request<UpdateRaceRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        let req = request.into_inner();

        let update = crate::models::RaceUpdate {
            source: req.source,
            title: req.title,
            state: req.state.and_then(|s| match ProtoRaceState::try_from(s) {
                Ok(ProtoRaceState::Queued) => Some(crate::models::RaceState::Queued),
                Ok(ProtoRaceState::Running) => Some(crate::models::RaceState::Running),
                Ok(ProtoRaceState::Passed) => Some(crate::models::RaceState::Passed),
                Ok(ProtoRaceState::Failed) => Some(crate::models::RaceState::Failed),
                Ok(ProtoRaceState::Canceled) => Some(crate::models::RaceState::Canceled),
                _ => None,
            }),
            started_at: req
                .started_at
                .and_then(|ts| chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)),
            eta_sec: req.eta_sec,
            progress: req.progress,
            deeplink: req.deeplink,
            metadata: if req.metadata.is_empty() {
                None
            } else {
                Some(req.metadata)
            },
            // New optimistic progress fields from request
            eta_source: req.eta_source.and_then(|s| raceboard::EtaSource::try_from(s).ok().map(|e| e as i32)),
            eta_confidence: req.eta_confidence,
            update_interval_hint: req.update_interval_hint,
        };

        match self.storage.update_race(&req.id, update).await {
            Some(race) => {
                // Persist completed races to historical store
                match race.state {
                    crate::models::RaceState::Passed
                    | crate::models::RaceState::Failed
                    | crate::models::RaceState::Canceled => {
                        use crate::persistence::RaceStore;
                        if let Err(e) = self.persistence.store_race(&race).await {
                            eprintln!("Failed to persist completed race {}: {}", race.id, e);
                        }
                        // Transitional: also update legacy JSON for visibility
                        let mut path =
                            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                        path.push(".raceboard");
                        let _ = std::fs::create_dir_all(&path);
                        path.push("races.json");
                        let mut races: Vec<crate::models::Race> = if path.exists() {
                            std::fs::read_to_string(&path)
                                .ok()
                                .and_then(|s| serde_json::from_str(&s).ok())
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        if let Some(existing) = races.iter_mut().find(|r| r.id == race.id) {
                            *existing = race.clone();
                        } else {
                            races.push(race.clone());
                        }
                        if let Ok(json) = serde_json::to_string_pretty(&races) {
                            let _ = std::fs::write(&path, json);
                        }
                    }
                    _ => {}
                }
                Ok(Response::new(convert_to_proto_race(race)))
            }
            None => Err(Status::not_found(format!("Race {} not found", req.id))),
        }
    }

    async fn add_event(
        &self,
        request: Request<AddEventRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        let req = request.into_inner();

        let event = req
            .event
            .ok_or_else(|| Status::invalid_argument("Event is required"))?;

        let model_event = crate::models::Event {
            event_type: event.r#type,
            data: event.data.and_then(|d| serde_json::from_str(&d).ok()),
            timestamp: event
                .timestamp
                .and_then(|ts| chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
                .unwrap_or_else(chrono::Utc::now),
        };

        match self
            .storage
            .add_event_to_race(&req.race_id, model_event)
            .await
        {
            Some(race) => Ok(Response::new(convert_to_proto_race(race))),
            None => Err(Status::not_found(format!("Race {} not found", req.race_id))),
        }
    }

    async fn list_races(&self, _request: Request<()>) -> Result<Response<RaceList>, Status> {
        let races = self.storage.get_all_races().await;
        Ok(Response::new(RaceList {
            races: races.into_iter().map(convert_to_proto_race).collect(),
        }))
    }

    async fn delete_race(
        &self,
        request: Request<DeleteRaceRequest>,
    ) -> Result<Response<()>, Status> {
        let id = request.into_inner().id;

        if self.storage.delete_race(&id).await {
            Ok(Response::new(()))
        } else {
            Err(Status::not_found(format!("Race {} not found", id)))
        }
    }

    async fn get_system_status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SystemStatus>, Status> {
        // Get adapter statuses from registry
        let adapters = self.adapter_registry.get_all().await;
        let summary = self.adapter_registry.get_summary().await;
        
        // Convert adapter statuses to proto format
        let mut proto_adapters = Vec::new();
        for (reg, health) in adapters {
            // Convert adapter type
            let adapter_type = match reg.adapter_type {
                crate::adapter_status::AdapterType::GitLab => ProtoAdapterType::Gitlab as i32,
                crate::adapter_status::AdapterType::Calendar => ProtoAdapterType::Calendar as i32,
                crate::adapter_status::AdapterType::CodexWatch => ProtoAdapterType::CodexWatch as i32,
                crate::adapter_status::AdapterType::GeminiWatch => ProtoAdapterType::GeminiWatch as i32,
                crate::adapter_status::AdapterType::Claude => ProtoAdapterType::Claude as i32,
                crate::adapter_status::AdapterType::Cmd => ProtoAdapterType::Cmd as i32,
            };
            
            // Convert health state
            let state = match health.state {
                crate::adapter_status::AdapterHealthState::Registered => ProtoHealthState::Registered as i32,
                crate::adapter_status::AdapterHealthState::Healthy => ProtoHealthState::Healthy as i32,
                crate::adapter_status::AdapterHealthState::Unhealthy => ProtoHealthState::Unhealthy as i32,
                crate::adapter_status::AdapterHealthState::Unknown => ProtoHealthState::Unknown as i32,
                crate::adapter_status::AdapterHealthState::Stopped => ProtoHealthState::Stopped as i32,
                crate::adapter_status::AdapterHealthState::Exempt => ProtoHealthState::Exempt as i32,
            };
            
            // Convert metrics
            let metrics = ProtoMetrics {
                races_created: health.metrics.races_created,
                races_updated: health.metrics.races_updated,
                last_activity: health.metrics.last_activity.map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                }),
                error_count: health.metrics.error_count,
                response_time_ms: health.metrics.response_time_ms.map(|v| v as i64),
                memory_usage_bytes: health.metrics.memory_usage_bytes,
                cpu_usage_percent: health.metrics.cpu_usage_percent,
            };
            
            proto_adapters.push(ProtoAdapterStatus {
                id: reg.id,
                adapter_type,
                instance_id: reg.instance_id,
                display_name: reg.display_name,
                version: reg.version,
                state,
                registered_at: Some(prost_types::Timestamp {
                    seconds: reg.registered_at.timestamp(),
                    nanos: reg.registered_at.timestamp_subsec_nanos() as i32,
                }),
                last_report: health.last_report.map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                }),
                seconds_since_report: health.seconds_since_report().map(|v| v as i64),
                health_interval_seconds: reg.health_interval_seconds,
                metrics: Some(metrics),
                error: health.error,
                pid: reg.pid,
                metadata: reg.metadata,
            });
        }
        
        // Count states
        let healthy_count = summary.state_counts.get(&crate::adapter_status::AdapterHealthState::Healthy)
            .copied().unwrap_or(0) as u32;
        let unhealthy_count = summary.state_counts.get(&crate::adapter_status::AdapterHealthState::Unhealthy)
            .copied().unwrap_or(0) as u32;
        let unknown_count = summary.state_counts.get(&crate::adapter_status::AdapterHealthState::Unknown)
            .copied().unwrap_or(0) as u32;
        let stopped_count = summary.state_counts.get(&crate::adapter_status::AdapterHealthState::Stopped)
            .copied().unwrap_or(0) as u32;
        let exempt_count = summary.state_counts.get(&crate::adapter_status::AdapterHealthState::Exempt)
            .copied().unwrap_or(0) as u32;
        
        let system_status = SystemStatus {
            adapters: proto_adapters,
            total_adapters: summary.total_adapters as u32,
            healthy_count,
            unhealthy_count,
            unknown_count,
            stopped_count,
            exempt_count,
            all_operational: summary.all_operational(),
            total_races_created: summary.total_races_created,
            total_races_updated: summary.total_races_updated,
            last_update: Some(prost_types::Timestamp {
                seconds: summary.last_update.timestamp(),
                nanos: summary.last_update.timestamp_subsec_nanos() as i32,
            }),
            cpu_usage_percent: None, // TODO: Add server metrics
            memory_usage_mb: None,
            active_races: Some(self.storage.get_all_races().await.len() as u64),
            server_uptime_seconds: None, // TODO: Track server start time
        };
        
        Ok(Response::new(system_status))
    }
}
