use crate::adapter_status::{AdapterStatusStore, AdapterStatus as AdapterStatusEnum, AdapterType};
use crate::grpc_service::raceboard::{
    ComponentHealth, ComponentState, ComponentStatus, SystemStatus,
    race_service_server::RaceService,
};
use chrono::Utc;
use prost_types::Timestamp;
use std::sync::Arc;
use std::time::SystemTime;
use tonic::{Request, Response, Status};
use tokio_stream::wrappers::ReceiverStream;
use tokio::sync::mpsc;

// Convert adapter status to proto
pub fn convert_adapter_status_to_proto(status: AdapterStatusEnum) -> ComponentState {
    match status {
        AdapterStatusEnum::Running => ComponentState::Running,
        AdapterStatusEnum::Stopped => ComponentState::Stopped,
        AdapterStatusEnum::Starting => ComponentState::Starting,
        AdapterStatusEnum::Stopping => ComponentState::Stopping,
        AdapterStatusEnum::Error => ComponentState::Error,
        AdapterStatusEnum::Unknown => ComponentState::Unknown,
    }
}

// Convert chrono DateTime to proto Timestamp
fn datetime_to_timestamp(dt: chrono::DateTime<chrono::Utc>) -> Option<Timestamp> {
    let system_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(dt.timestamp() as u64);
    Some(Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    })
}

// Implementation of status methods for RaceServiceImpl
impl crate::grpc_service::RaceServiceImpl {
    pub async fn get_system_status_impl(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SystemStatus>, Status> {
        let adapters = self.adapter_status.get_all().await;
        let summary = self.adapter_status.get_summary().await;
        
        // Convert adapter infos to proto
        let adapter_statuses: Vec<ComponentStatus> = adapters
            .into_iter()
            .map(|info| {
                ComponentStatus {
                    id: info.adapter_type.as_str().to_string(),
                    display_name: info.display_name,
                    state: convert_adapter_status_to_proto(info.status) as i32,
                    health: Some(ComponentHealth {
                        healthy: info.health.healthy,
                        last_check: datetime_to_timestamp(info.health.last_check),
                        response_time_ms: info.health.response_time_ms.map(|v| v as i64),
                        error_message: info.health.error_message,
                        consecutive_failures: info.health.consecutive_failures,
                    }),
                    pid: info.pid,
                    started_at: info.started_at.and_then(datetime_to_timestamp),
                    port: info.port,
                    version: info.version,
                    config_path: info.config_path,
                    last_activity: info.last_activity.and_then(datetime_to_timestamp),
                    races_created: info.races_created,
                    races_updated: info.races_updated,
                    metadata: info.metadata,
                    uptime_seconds: info.uptime_seconds(),
                }
            })
            .collect();

        // Get server health metrics
        let server_start_time = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() - 3600; // Dummy: assume server started 1 hour ago
        
        let server_status = ComponentStatus {
            id: "server".to_string(),
            display_name: "Raceboard Server".to_string(),
            state: ComponentState::Running as i32,
            health: Some(ComponentHealth {
                healthy: true,
                last_check: datetime_to_timestamp(Utc::now()),
                response_time_ms: Some(1),
                error_message: None,
                consecutive_failures: 0,
            }),
            pid: Some(std::process::id()),
            started_at: Some(Timestamp {
                seconds: server_start_time as i64,
                nanos: 0,
            }),
            port: Some(7777),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            config_path: None,
            last_activity: datetime_to_timestamp(Utc::now()),
            races_created: 0,
            races_updated: 0,
            metadata: std::collections::HashMap::new(),
            uptime_seconds: Some(3600),
        };

        let storage = self.storage.clone();
        let active_races = storage.get_all_races().await.len() as u64;

        let system_status = SystemStatus {
            server: Some(server_status),
            adapters: adapter_statuses,
            total_adapters: summary.total_adapters as u32,
            running_adapters: summary.running as u32,
            stopped_adapters: summary.stopped as u32,
            errored_adapters: summary.errored as u32,
            healthy_adapters: summary.healthy as u32,
            total_races_created: summary.total_races_created,
            total_races_updated: summary.total_races_updated,
            last_update: datetime_to_timestamp(summary.last_update),
            cpu_usage_percent: None, // TODO: Implement CPU monitoring
            memory_usage_mb: None,   // TODO: Implement memory monitoring
            active_races: Some(active_races),
            server_uptime_seconds: Some(3600), // TODO: Track actual uptime
        };

        Ok(Response::new(system_status))
    }

    pub async fn stream_component_status_impl(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ReceiverStream<Result<ComponentStatus, Status>>>, Status> {
        let (tx, rx) = mpsc::channel(100);
        let adapter_status = self.adapter_status.clone();

        // Start a task that periodically sends status updates
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            
            loop {
                interval.tick().await;
                
                let adapters = adapter_status.get_all().await;
                
                for info in adapters {
                    let status = ComponentStatus {
                        id: info.adapter_type.as_str().to_string(),
                        display_name: info.display_name,
                        state: convert_adapter_status_to_proto(info.status) as i32,
                        health: Some(ComponentHealth {
                            healthy: info.health.healthy,
                            last_check: datetime_to_timestamp(info.health.last_check),
                            response_time_ms: info.health.response_time_ms.map(|v| v as i64),
                            error_message: info.health.error_message,
                            consecutive_failures: info.health.consecutive_failures,
                        }),
                        pid: info.pid,
                        started_at: info.started_at.and_then(datetime_to_timestamp),
                        port: info.port,
                        version: info.version,
                        config_path: info.config_path,
                        last_activity: info.last_activity.and_then(datetime_to_timestamp),
                        races_created: info.races_created,
                        races_updated: info.races_updated,
                        metadata: info.metadata,
                        uptime_seconds: info.uptime_seconds(),
                    };
                    
                    if tx.send(Ok(status)).await.is_err() {
                        // Client disconnected
                        break;
                    }
                }
                
                if tx.is_closed() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}