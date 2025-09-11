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
    AdapterType as ProtoAdapterType, AdapterMetrics as ProtoMetrics, EtaRevision as ProtoEtaRevision,
};

pub struct RaceServiceImpl {
    storage: Arc<Storage>,
    persistence: Arc<PersistenceLayer>,
    adapter_registry: Arc<AdapterRegistry>,
    read_only: bool,
}

impl RaceServiceImpl {
    pub fn new(
        storage: Arc<Storage>,
        persistence: Arc<PersistenceLayer>,
        adapter_registry: Arc<AdapterRegistry>,
        read_only: bool,
    ) -> Self {
        Self {
            storage,
            persistence,
            adapter_registry,
            read_only,
        }
    }
}

// Convert our internal Race to proto Race
fn race_to_proto(race: &crate::models::Race) -> ProtoRace {
    ProtoRace {
        id: race.id.clone(),
        source: race.source.clone(),
        title: race.title.clone(),
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
        deeplink: race.deeplink.clone(),
        metadata: race.metadata.clone().unwrap_or_default(),
        events: race
            .events
            .as_ref()
            .map(|events| events
                .iter()
                .map(|e| ProtoEvent {
                    r#type: e.event_type.clone(),
                    data: e.data.as_ref().and_then(|v| v.as_str()).map(|s| s.to_string()),
                    timestamp: Some(prost_types::Timestamp {
                        seconds: e.timestamp.timestamp(),
                        nanos: e.timestamp.timestamp_subsec_nanos() as i32,
                    }),
                })
                .collect())
            .unwrap_or_default(),
        last_progress_update: race.last_progress_update.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        }),
        last_eta_update: race.last_eta_update.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        }),
        eta_source: race.eta_source.map(|v| v as i32),
        eta_confidence: race.eta_confidence,
        update_interval_hint: race.update_interval_hint,
        eta_history: race
            .eta_history
            .as_ref()
            .map(|hist| {
                hist.iter()
                    .map(|rev| ProtoEtaRevision {
                        eta_sec: rev.eta_sec,
                        timestamp: Some(prost_types::Timestamp {
                            seconds: rev.timestamp.timestamp(),
                            nanos: rev.timestamp.timestamp_subsec_nanos() as i32,
                        }),
                        source: rev.source,
                        confidence: rev.confidence,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

// Convert proto Race to our internal Race
fn proto_to_race(proto: ProtoRace) -> crate::models::Race {
    let mut race = crate::models::Race {
        id: proto.id,
        source: proto.source,
        title: proto.title,
        state: match proto.state {
            x if x == ProtoRaceState::Queued as i32 => crate::models::RaceState::Queued,
            x if x == ProtoRaceState::Running as i32 => crate::models::RaceState::Running,
            x if x == ProtoRaceState::Passed as i32 => crate::models::RaceState::Passed,
            x if x == ProtoRaceState::Failed as i32 => crate::models::RaceState::Failed,
            x if x == ProtoRaceState::Canceled as i32 => crate::models::RaceState::Canceled,
            _ => crate::models::RaceState::Queued,
        },
        started_at: proto
            .started_at
            .map(|ts| {
                chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                    .unwrap_or_else(chrono::Utc::now)
            })
            .unwrap_or_else(chrono::Utc::now),
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
            Some(proto
                .events
                .into_iter()
                .map(|e| crate::models::Event {
                    event_type: e.r#type,
                    data: e.data.map(|s| serde_json::Value::String(s)),
                    timestamp: e
                        .timestamp
                        .map(|ts| {
                            chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                                .unwrap_or_else(chrono::Utc::now)
                        })
                        .unwrap_or_else(chrono::Utc::now),
                })
                .collect())
        },
        last_progress_update: proto.last_progress_update.and_then(|ts| {
            chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
        }),
        last_eta_update: proto.last_eta_update.and_then(|ts| {
            chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
        }),
        eta_source: proto.eta_source,
        eta_confidence: proto.eta_confidence,
        update_interval_hint: proto.update_interval_hint,
        completed_at: None,
        duration_sec: None,
        eta_history: if proto.eta_history.is_empty() {
            None
        } else {
            Some(
                proto
                    .eta_history
                    .into_iter()
                    .filter_map(|rev| {
                        let ts = rev.timestamp.and_then(|t| {
                            chrono::DateTime::from_timestamp(t.seconds, t.nanos as u32)
                        });
                        ts.map(|timestamp| crate::models::EtaRevision {
                            eta_sec: rev.eta_sec,
                            timestamp,
                            source: rev.source,
                            confidence: rev.confidence,
                        })
                    })
                    .collect(),
            )
        },
    };
    race.infer_eta_source();
    race.infer_eta_confidence();
    race.infer_update_interval_hint();
    race
}

#[tonic::async_trait]
impl RaceService for RaceServiceImpl {
    type StreamRacesStream = std::pin::Pin<Box<dyn futures::Stream<Item = Result<RaceUpdate, Status>> + Send>>;

    async fn stream_races(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::StreamRacesStream>, Status> {
        // Subscribe to storage events
        let mut rx = self.storage.subscribe();
        
        // Create a channel for race updates
        let (tx, _) = tokio::sync::broadcast::channel(100);
        let tx_clone = tx.clone();
        
        // Spawn a task to convert storage events to race updates
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                let update = match event {
                    StorageEvent::Created(race) => RaceUpdate {
                        r#type: race_update::UpdateType::Created as i32,
                        race: Some(race_to_proto(&race)),
                    },
                    StorageEvent::Updated(race) => RaceUpdate {
                        r#type: race_update::UpdateType::Updated as i32,
                        race: Some(race_to_proto(&race)),
                    },
                    StorageEvent::Deleted(id) => RaceUpdate {
                        r#type: race_update::UpdateType::Deleted as i32,
                        race: Some(ProtoRace {
                            id,
                            ..Default::default()
                        }),
                    },
                };
                
                let _ = tx_clone.send(update);
            }
        });
        
        // Return the stream
        use futures::StreamExt;
        let broadcast_stream = tokio_stream::wrappers::BroadcastStream::new(tx.subscribe());
        let mapped_stream = broadcast_stream.filter_map(|result| async move {
            match result {
                Ok(update) => Some(Ok(update)),
                Err(_) => None,
            }
        });
        
        Ok(Response::new(Box::pin(mapped_stream)))
    }

    async fn get_race(
        &self,
        request: Request<GetRaceRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        let id = request.into_inner().id;
        
        match self.storage.get_race(&id).await {
            Some(race) => Ok(Response::new(race_to_proto(&race))),
            None => Err(Status::not_found(format!("Race {} not found", id))),
        }
    }

    async fn create_race(&self, request: Request<ProtoRace>) -> Result<Response<ProtoRace>, Status> {
        if self.read_only {
            return Err(Status::permission_denied("Server is in read-only mode"));
        }
        let proto_race = request.into_inner();
        let race = proto_to_race(proto_race);
        
        // Reject adapter registrations - use REST adapter endpoints instead
        if crate::models::is_adapter_id(&race.id) {
            return Err(Status::invalid_argument(
                "Adapter registrations must use REST endpoints (/adapter/register), not CreateRace"
            ));
        }
        
        // Store the race
        self.storage.create_or_update_race(race.clone()).await;
        
        // Persist if this is a completed race
        if matches!(
            race.state,
            crate::models::RaceState::Passed
                | crate::models::RaceState::Failed
                | crate::models::RaceState::Canceled
        ) {
            use crate::persistence::RaceStore;
            let _ = self.persistence.store_race(&race).await;
        }
        
        Ok(Response::new(race_to_proto(&race)))
    }

    async fn update_race(
        &self,
        request: Request<UpdateRaceRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        if self.read_only {
            return Err(Status::permission_denied("Server is in read-only mode"));
        }
        let update_req = request.into_inner();
        let id = update_req.id.clone();
        
        // Build update from request
        let update = crate::models::RaceUpdate {
            source: update_req.source,
            title: update_req.title,
            state: update_req.state.map(|s| match s {
                x if x == ProtoRaceState::Queued as i32 => crate::models::RaceState::Queued,
                x if x == ProtoRaceState::Running as i32 => crate::models::RaceState::Running,
                x if x == ProtoRaceState::Passed as i32 => crate::models::RaceState::Passed,
                x if x == ProtoRaceState::Failed as i32 => crate::models::RaceState::Failed,
                x if x == ProtoRaceState::Canceled as i32 => crate::models::RaceState::Canceled,
                _ => crate::models::RaceState::Queued,
            }),
            started_at: update_req.started_at.and_then(|ts| {
                chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
            }),
            eta_sec: update_req.eta_sec,
            progress: update_req.progress,
            deeplink: update_req.deeplink,
            metadata: if update_req.metadata.is_empty() {
                None
            } else {
                Some(update_req.metadata)
            },
            eta_source: update_req.eta_source,
            eta_confidence: update_req.eta_confidence,
            update_interval_hint: update_req.update_interval_hint,
        };
        
        // Apply update
        // NOTE: Adapter health/registration must NOT be inferred from race updates.
        // Adapter lifecycle is handled via dedicated RPCs (RegisterAdapter, ReportAdapterHealth, DeregisterAdapter)
        // or REST endpoints; race updates are unrelated.
        match self.storage.update_race(&id, update).await {
            Some(race) => {
                // Persist if this is now completed
                if matches!(
                    race.state,
                    crate::models::RaceState::Passed
                        | crate::models::RaceState::Failed
                        | crate::models::RaceState::Canceled
                ) {
                    use crate::persistence::RaceStore;
                    let _ = self.persistence.store_race(&race).await;
                }
                
                Ok(Response::new(race_to_proto(&race)))
            }
            None => Err(Status::not_found(format!("Race {} not found", id))),
        }
    }

    async fn add_event(
        &self,
        request: Request<AddEventRequest>,
    ) -> Result<Response<ProtoRace>, Status> {
        if self.read_only {
            return Err(Status::permission_denied("Server is in read-only mode"));
        }
        let req = request.into_inner();
        let race_id = req.race_id;
        let event = req.event.ok_or_else(|| Status::invalid_argument("Event is required"))?;
        
        let internal_event = crate::models::Event {
            event_type: event.r#type,
            data: event.data.map(|s| serde_json::Value::String(s)),
            timestamp: event
                .timestamp
                .map(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .unwrap_or_else(chrono::Utc::now)
                })
                .unwrap_or_else(chrono::Utc::now),
        };
        
        match self.storage.add_event_to_race(&race_id, internal_event).await {
            Some(race) => Ok(Response::new(race_to_proto(&race))),
            None => Err(Status::not_found(format!("Race {} not found", race_id))),
        }
    }

    async fn list_races(&self, _request: Request<()>) -> Result<Response<RaceList>, Status> {
        let races = self.storage.get_all_races().await;
        // Filter out adapter registrations (IDs starting with "adapter:")
        let proto_races = races
            .iter()
            .filter(|r| !r.id.starts_with("adapter:"))
            .map(race_to_proto)
            .collect();
        
        Ok(Response::new(RaceList {
            races: proto_races,
        }))
    }

    async fn delete_race(
        &self,
        request: Request<DeleteRaceRequest>,
    ) -> Result<Response<()>, Status> {
        if self.read_only {
            return Err(Status::permission_denied("Server is in read-only mode"));
        }
        let id = request.into_inner().id;

        // Adapter deregistration must not be tied to race deletions; use dedicated RPCs/REST.
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
        // Get all races (for active_races only)
        let all_races = self.storage.get_all_races().await;
        
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
        
        // Total adapters are determined solely by the registry
        let total_adapters = proto_adapters.len() as u32;
        
        let system_status = SystemStatus {
            adapters: proto_adapters,
            total_adapters,
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
            active_races: Some(all_races.len() as u64),
            server_uptime_seconds: None, // TODO: Track server start time
        };
        
        Ok(Response::new(system_status))
    }
    
}