mod adapter_status;
mod app_state;
mod cluster;
mod config;
mod grpc_service;
mod handlers;
mod hnsw_dbscan;
mod models;
mod monitoring;
mod persistence;
mod phased_rollout;
mod prediction;
mod processing;
mod rebuild;
mod rebuild_trigger;
mod stats;
mod storage;
#[cfg(test)]
mod tests;

use actix_web::dev::ServerHandle;
use actix_web::{middleware, web, App, HttpServer};
use app_state::AppState;
use cluster::ClusteringEngine;
use config::Settings;
use grpc_service::raceboard::race_service_server::RaceServiceServer;
use grpc_service::RaceServiceImpl;
use persistence::{PersistenceLayer, RaceStore};
use prediction::PredictionEngine;
use processing::ProcessingEngine;
use rebuild::{DoubleBufferClusters, RebuildConfig};
use rebuild_trigger::RebuildTrigger;
use std::sync::Arc;
use storage::Storage;
use tokio::signal;
use tonic::transport::Server as GrpcServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args: Vec<String> = std::env::args().collect();
    let clear_clusters = args.contains(&"--clear-clusters".to_string());
    
    if clear_clusters {
        log::info!("Starting with --clear-clusters flag: will clear all clusters on startup");
    }
    
    // Load configuration
    let settings = Settings::new()?;

    // Initialize logging
    env_logger::init_from_env(env_logger::Env::new().default_filter_or(&settings.logging.level));

    log::info!("Starting Raceboard Server");
    log::info!("HTTP: http://{}", settings.http_addr());
    log::info!("gRPC: grpc://{}", settings.grpc_addr());

    // Initialize storage with config
    // CRITICAL: Use much larger limits to support cluster rebuilding
    let max_races = settings.storage.max_races.max(100_000); // Minimum 100k for ML
    let max_events = settings.storage.max_events_per_race.max(1000);

    log::info!(
        "Storage configuration: max_races={}, max_events_per_race={}",
        max_races,
        max_events
    );
    let storage = Arc::new(Storage::with_config(max_races, max_events));

    // Initialize monitoring system
    let monitoring = Arc::new(monitoring::MonitoringSystem::new(max_races));
    let alert_system = Arc::new(monitoring::AlertSystem::new(None)); // Configure webhook URL if needed

    // Start monitoring - but we need persistence first, so we'll start it later
    log::info!("Monitoring system initialized");

    // Initialize ETA prediction system
    log::info!("Initializing ETA prediction system...");
    let clustering_engine = Arc::new(ClusteringEngine::new(1000));

    // Try to initialize persistence, but continue if it fails
    let persistence = match PersistenceLayer::new(None) {
        Ok(p) => {
            log::info!("Persistence layer initialized");
            Arc::new(p)
        }
        Err(e) => {
            log::warn!(
                "Failed to initialize persistence layer: {}. Running without persistence.",
                e
            );
            // Create a dummy persistence layer that uses an in-memory database
            match PersistenceLayer::new(Some(std::path::PathBuf::from("/tmp/raceboard_temp.db"))) {
                Ok(p) => Arc::new(p),
                Err(_) => {
                    // Last resort: use in-memory sled
                    Arc::new(PersistenceLayer::new_in_memory()?)
                }
            }
        }
    };

    // Do initial data check for accurate stats
    monitoring.check_initial_data(&storage, &persistence).await;

    // Start monitoring now that we have persistence
    monitoring
        .clone()
        .start_monitoring(storage.clone(), persistence.clone())
        .await;
    log::info!("Storage monitoring started");

    // Load existing clusters from disk (unless --clear-clusters flag is set)
    if clear_clusters {
        log::info!("Clearing all clusters due to --clear-clusters flag");
        // Clear persisted clusters
        if let Err(e) = persistence.clear_clusters() {
            log::error!("Failed to clear clusters: {}", e);
        } else {
            log::info!("Cleared persisted clusters");
        }
    } else {
        if let Ok(saved_clusters) = persistence.load_clusters() {
            if !saved_clusters.is_empty() {
                let mut clusters = clustering_engine.clusters.write().await;
                for (id, cluster) in saved_clusters {
                    clusters.insert(id, cluster);
                }
                log::info!("Loaded {} clusters from disk", clusters.len());
            }
        }
    }

    // Bootstrap: import existing JSON history into sled on first run only
    {
        // Check if we already have data in sled
        let existing_count = persistence.races_count();
        let migration_complete = persistence.is_migration_complete();

        if existing_count > 0 || migration_complete {
            log::info!(
                "Skipping JSON migration: {} races already in sled (migration_complete={})",
                existing_count,
                migration_complete
            );
        } else {
            // Perform one-time migration
            let mut path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            path.push(".raceboard");
            path.push("races.json");
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(contents) => {
                        match serde_json::from_str::<Vec<crate::models::Race>>(&contents) {
                            Ok(races) => {
                                if !races.is_empty() {
                                    log::info!(
                                        "One-time migration: importing {} JSON races into sled...",
                                        races.len()
                                    );
                                    let start = std::time::Instant::now();
                                    let mut upserted = 0usize;
                                    for race in races {
                                        if let Err(e) = persistence.store_race(&race).await {
                                            log::error!("Failed to import race {}: {}", race.id, e);
                                        } else {
                                            upserted += 1;
                                        }
                                    }
                                    let elapsed = start.elapsed();
                                    log::info!(
                                        "Migration complete: imported {} races in {:.2}s",
                                        upserted,
                                        elapsed.as_secs_f32()
                                    );

                                    // Mark migration as complete
                                    if let Err(e) = persistence.mark_migration_complete() {
                                        log::error!("Failed to mark migration complete: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to parse JSON races at {:?}: {}", path, e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to read JSON races at {:?}: {}", path, e);
                    }
                }
            } else {
                log::info!("No JSON races file found at {:?}, skipping migration", path);
                // Still mark as complete to avoid checking every time
                if let Err(e) = persistence.mark_migration_complete() {
                    log::error!("Failed to mark migration complete: {}", e);
                }
            }
        }
    }

    // Do not migrate active in-memory races; persistence is for historical (completed) data only.

    let prediction_engine = Arc::new(PredictionEngine::new(
        clustering_engine.clone(),
        persistence.clone(),
    ));

    let processing_engine = Arc::new(ProcessingEngine::new(prediction_engine.clone()));

    // Initialize rebuild system
    let rebuild_config = RebuildConfig::default();
    let rebuild_clusters = Arc::new(DoubleBufferClusters::new(100_000_000)); // 100MB baseline

    // Initialize active buffer with current clusters (unless --clear-clusters is set)
    if !clear_clusters {
        let mut active = rebuild_clusters.active.write().await;
        let current_clusters = clustering_engine.clusters.read().await;
        *active = current_clusters.clone();
    } else {
        log::info!("Not loading clusters into rebuild buffer due to --clear-clusters flag");
    }

    log::info!("Creating RebuildTrigger...");
    let rebuild_trigger = Arc::new(RebuildTrigger::new(
        rebuild_config,
        rebuild_clusters.clone(),
        persistence.clone(),
        clustering_engine.clone(),
    ));

    // Start rebuild monitoring
    log::info!("Starting rebuild monitoring...");
    rebuild_trigger.clone().start_monitoring().await;
    log::info!("Cluster rebuild system initialized");

    // Initialize data layer metrics
    let data_layer_metrics = Arc::new(monitoring::DataLayerMetrics::new());

    // Initialize adapter registry
    let adapter_registry = Arc::new(adapter_status::AdapterRegistry::new());
    log::info!("Adapter registry initialized");

    // Start adapter monitoring background job
    let monitor = adapter_status::AdapterMonitor::new((*adapter_registry).clone());
    tokio::spawn(async move {
        log::info!("Starting adapter health monitoring");
        monitor.run().await;
    });

    let app_state = AppState {
        storage: storage.clone(),
        prediction_engine: prediction_engine.clone(),
        processing_engine: processing_engine.clone(),
        rebuild_clusters: rebuild_clusters.clone(),
        rebuild_trigger: rebuild_trigger.clone(),
        persistence: persistence.clone(),
        monitoring: monitoring.clone(),
        alert_system: alert_system.clone(),
        data_layer_metrics: Some(data_layer_metrics.clone()),
        adapter_registry: adapter_registry.clone(),
        read_only: settings.server.read_only
            || std::env::var("RACEBOARD_READ_ONLY")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        legacy_json_fallback_enabled: settings.server.legacy_json_fallback_enabled
            || std::env::var("RACEBOARD_SERVER__LEGACY_JSON_FALLBACK_ENABLED")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
    };
    log::info!("Storage: in-memory with ETA prediction and cluster rebuilding");

    // Start HTTP server within current Tokio runtime
    let http_settings = settings.clone();
    let http_state = app_state.clone();
    let http_server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(http_state.clone()))
            .wrap(middleware::Logger::default())
            .wrap(
                middleware::DefaultHeaders::new()
                    .add(("Access-Control-Allow-Origin", "*"))
                    .add((
                        "Access-Control-Allow-Methods",
                        "GET, POST, PATCH, DELETE, OPTIONS",
                    ))
                    .add(("Access-Control-Allow-Headers", "Content-Type")),
            )
            .service(web::resource("/health").route(web::get().to(handlers::health_check)))
            .service(web::resource("/races").route(web::get().to(handlers::get_races)))
            .service(web::resource("/race").route(web::post().to(handlers::create_race)))
            .service(
                web::resource("/race/{id}")
                    .route(web::get().to(handlers::get_race))
                    .route(web::patch().to(handlers::update_race))
                    .route(web::delete().to(handlers::delete_race)),
            )
            .service(web::resource("/race/{id}/event").route(web::post().to(handlers::add_event)))
            .service(web::resource("/clusters").route(web::get().to(handlers::get_clusters)))
            .service(web::resource("/cluster/{id}").route(web::get().to(handlers::get_cluster)))
            // Historic data management
            .service(
                web::resource("/historic/races").route(web::get().to(handlers::get_historic_races)),
            )
            .service(web::resource("/admin/purge").route(web::post().to(handlers::admin_purge)))
            .service(web::resource("/admin/compact").route(web::post().to(handlers::admin_compact)))
            .service(
                web::resource("/admin/storage-report")
                    .route(web::get().to(handlers::admin_storage_report)),
            )
            .service(web::resource("/admin/metrics").route(web::get().to(handlers::admin_metrics)))
            // Rebuild metrics and debug endpoints
            .service(
                web::resource("/metrics/rebuild")
                    .route(web::get().to(handlers::get_rebuild_metrics)),
            )
            .service(
                web::resource("/metrics/rollout")
                    .route(web::get().to(handlers::get_rollout_status)),
            )
            .service(
                web::resource("/rollout/enable_all")
                    .route(web::post().to(handlers::enable_all_sources)),
            )
            .service(
                web::resource("/rebuild/trigger").route(web::post().to(handlers::trigger_rebuild)),
            )
            .service(
                web::resource("/rollout/reset")
                    .route(web::post().to(handlers::reset_rollout_phase)),
            )
            .service(
                web::resource("/debug/cluster/{id}")
                    .route(web::get().to(handlers::get_cluster_debug)),
            )
            // Adapter status endpoints
            .service(
                web::resource("/adapter/register")
                    .route(web::post().to(handlers::adapter_register)),
            )
            .service(
                web::resource("/adapter/health")
                    .route(web::post().to(handlers::adapter_health)),
            )
            .service(
                web::resource("/adapter/deregister")
                    .route(web::post().to(handlers::adapter_deregister)),
            )
            .service(
                web::resource("/adapter/status")
                    .route(web::get().to(handlers::get_adapter_status)),
            )
            .service(
                web::resource("/adapter/metrics")
                    .route(web::get().to(handlers::get_adapter_metrics)),
            )
    })
    .bind(http_settings.http_addr())
    .unwrap()
    .shutdown_timeout(5)
    .run();

    let http_handle: ServerHandle = http_server.handle();
    let http_task = tokio::spawn(http_server);

    // Start gRPC server with graceful shutdown
    let grpc_settings = settings.clone();
    let grpc_service = RaceServiceImpl::new(storage.clone(), persistence.clone(), adapter_registry.clone(), settings.server.read_only);
    let (grpc_shutdown_tx, grpc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let grpc_task = tokio::spawn(async move {
        let addr = grpc_settings.grpc_addr().parse().unwrap();
        log::info!("gRPC server listening on {}", addr);
        GrpcServer::builder()
            .add_service(RaceServiceServer::new(grpc_service))
            .serve_with_shutdown(addr, async move {
                let _ = grpc_shutdown_rx.await;
            })
            .await
    });

    // Start scheduled snapshot task (daily at 00:00 UTC)
    let snapshot_persistence = persistence.clone();
    let (snapshot_shutdown_tx, mut snapshot_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let snapshot_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // Check every hour
        let mut last_snapshot_day = 0;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    use chrono::Datelike;
                    use chrono::Timelike;

                    let now = chrono::Utc::now();
                    let current_day = now.ordinal();

                    // Take snapshot at midnight UTC (or on first run)
                    if current_day != last_snapshot_day && (now.hour() == 0 || last_snapshot_day == 0) {
                        log::info!("Starting daily JSON snapshot...");
                        match snapshot_persistence.create_json_snapshot().await {
                            Ok(_) => {
                                log::info!("Daily JSON snapshot completed successfully");
                                last_snapshot_day = current_day;
                            }
                            Err(e) => {
                                log::error!("Failed to create JSON snapshot: {}", e);
                            }
                        }
                    }
                }
                _ = &mut snapshot_shutdown_rx => {
                    log::info!("Snapshot task shutting down");
                    break;
                }
            }
        }
    });

    // Wait for Ctrl-C
    log::info!("Press Ctrl-C to stop");
    match signal::ctrl_c().await {
        Ok(()) => {
            log::info!("Shutdown signal received, stopping servers...");
        }
        Err(e) => {
            log::error!("Failed to listen for shutdown signal: {}", e);
        }
    }

    // Trigger shutdown
    let _ = grpc_shutdown_tx.send(());
    let _ = snapshot_shutdown_tx.send(());
    let stop_fut = http_handle.stop(true);
    // Best-effort graceful stop within a timeout
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), stop_fut).await;

    // Await tasks
    if let Err(e) = grpc_task.await {
        log::error!("gRPC server error: {:?}", e);
    }
    if let Err(e) = http_task.await {
        log::error!("HTTP server task error: {:?}", e);
    }
    if let Err(e) = snapshot_task.await {
        log::error!("Snapshot task error: {:?}", e);
    }

    Ok(())
}
