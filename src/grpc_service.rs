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
};

pub struct RaceServiceImpl {
    storage: Arc<Storage>,
    persistence: Arc<PersistenceLayer>,
}

impl RaceServiceImpl {
    pub fn new(storage: Arc<Storage>, persistence: Arc<PersistenceLayer>) -> Self {
        Self {
            storage,
            persistence,
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
}
