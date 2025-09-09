use crate::phased_rollout::RolloutMode;
use crate::{
    app_state::AppState,
    models::{Event, Race, RaceState, RaceUpdate},
    processing::RaceProcessingRequest,
};
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub async fn get_races(data: web::Data<AppState>) -> Result<HttpResponse> {
    let races = data.storage.get_all_races().await;
    Ok(HttpResponse::Ok().json(races))
}

pub async fn create_race(race: web::Json<Race>, data: web::Data<AppState>) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    let mut race = race.into_inner();

    // Infer ETA source and confidence if not provided
    if race.eta_source.is_none() && race.eta_sec.is_some() {
        race.eta_source = Some(match race.source.as_str() {
            "google-calendar" => 1, // EtaSource::Exact
            "gitlab" | "github" | "jenkins" => 2, // EtaSource::Adapter
            _ => 2, // Default to Adapter
        });
    }

    // Only predict ETA if not already provided by the adapter
    if race.eta_sec.is_none() {
        let metadata = race.metadata.clone().unwrap_or_default();
        let eta_prediction = data
            .prediction_engine
            .predict_eta(&race.id, &race.title, &race.source, &metadata)
            .await;

        race.eta_sec = Some(eta_prediction.expected_seconds);
        race.eta_source = Some(3); // EtaSource::Cluster
        race.eta_confidence = Some(0.7); // Cluster predictions have 70% confidence
    }

    // Infer confidence based on source if not set
    if race.eta_confidence.is_none() && race.eta_source.is_some() {
        race.eta_confidence = Some(match race.eta_source.unwrap() {
            1 => 1.0,   // Exact
            3 => 0.7,   // Cluster
            2 => 0.5,   // Adapter
            4 => 0.2,   // Bootstrap
            _ => 0.3,
        });
    }

    // Infer update_interval_hint based on source if not set
    if race.update_interval_hint.is_none() && race.eta_source.is_some() {
        race.update_interval_hint = Some(match race.eta_source.unwrap() {
            1 => 60,    // Exact: 60s
            2 => 10,    // Adapter: 10s
            3 => 15,    // Cluster: 15s
            4 => 10,    // Bootstrap: 10s
            _ => 10,
        });
    }

    // Store the race in memory (UI/gRPC hot path only; no persistence at creation)
    let race = data.storage.create_or_update_race(race).await;

    Ok(HttpResponse::Ok().json(race))
}

pub async fn get_race(path: web::Path<String>, data: web::Data<AppState>) -> Result<HttpResponse> {
    let id = path.into_inner();

    match data.storage.get_race(&id).await {
        Some(race) => Ok(HttpResponse::Ok().json(race)),
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Race not found",
            "id": id
        }))),
    }
}

pub async fn update_race(
    path: web::Path<String>,
    race_update: web::Json<RaceUpdate>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    let id = path.into_inner();
    let update = race_update.into_inner();

    // If race is completing, calculate duration and update stats
    if let Some(ref state) = update.state {
        if matches!(
            state,
            RaceState::Passed | RaceState::Failed | RaceState::Canceled
        ) {
            if let Some(existing_race) = data.storage.get_race(&id).await {
                let duration = chrono::Utc::now()
                    .signed_duration_since(existing_race.started_at)
                    .num_seconds();

                let metadata = existing_race.metadata.clone().unwrap_or_default();

                // Submit to processing engine for stats update
                let request = RaceProcessingRequest {
                    race_id: existing_race.id.clone(),
                    race_title: existing_race.title.clone(),
                    race_source: existing_race.source.clone(),
                    race_metadata: metadata,
                    duration: Some(duration),
                };

                let _ = data.processing_engine.submit_race(request).await;
            }
        }
    }

    match data.storage.update_race(&id, update).await {
        Some(race) => {
            // Persist completed races only (historical store)
            if matches!(
                race.state,
                RaceState::Passed | RaceState::Failed | RaceState::Canceled
            ) {
                log::warn!(
                    "HANDLER: Race {} completed with state {:?}, persisting to sled",
                    race.id,
                    race.state
                );
                use crate::persistence::RaceStore;
                if let Err(e) = data.persistence.store_race(&race).await {
                    log::error!("Failed to persist completed race {}: {}", race.id, e);
                    eprintln!("Failed to persist completed race {}: {}", race.id, e);
                } else {
                    log::warn!("HANDLER: Successfully persisted race {} to sled", race.id);
                }
                // Transitional: also update legacy JSON for visibility until full sled migration is stable
                let mut path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                path.push(".raceboard");
                let _ = std::fs::create_dir_all(&path);
                path.push("races.json");
                let mut races: Vec<Race> = if path.exists() {
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
            Ok(HttpResponse::Ok().json(race))
        }
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Race not found",
            "id": id
        }))),
    }
}

pub async fn add_event(
    path: web::Path<String>,
    event: web::Json<Event>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    let id = path.into_inner();

    match data
        .storage
        .add_event_to_race(&id, event.into_inner())
        .await
    {
        Some(race) => {
            // Events are part of hot path; do not persist here
            Ok(HttpResponse::Ok().json(race))
        }
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Race not found",
            "id": id
        }))),
    }
}

pub async fn delete_race(
    path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    let id = path.into_inner();

    if data.storage.delete_race(&id).await {
        // Do not delete from persistence; historical data is retained
        Ok(HttpResponse::Ok().json(json!({
            "message": "Race deleted successfully",
            "id": id
        })))
    } else {
        Ok(HttpResponse::NotFound().json(json!({
            "error": "Race not found",
            "id": id
        })))
    }
}

pub async fn health_check(data: web::Data<AppState>) -> Result<HttpResponse> {
    let storage_health = data.monitoring.get_health().await;

    let status = if storage_health.critical_errors.is_empty() {
        "healthy"
    } else {
        "critical"
    };

    Ok(HttpResponse::Ok().json(json!({
        "status": status,
        "service": "Raceboard Server",
        "version": "1.0.0",
        "read_only_mode_active": data.read_only,
        "storage": {
            "total_races": storage_health.total_races,
            "max_races": storage_health.max_races,
            "usage_percent": storage_health.usage_percent,
            "eviction_count": storage_health.eviction_count,
            "cluster_data_sufficient": storage_health.cluster_data_sufficient,
            "warnings": storage_health.warnings,
            "critical_errors": storage_health.critical_errors,
        }
    })))
}

pub async fn get_clusters(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Get all clusters from the clustering engine
    let clusters = data
        .prediction_engine
        .clustering_engine
        .clusters
        .read()
        .await;

    // Convert to a serializable format
    let cluster_list: Vec<serde_json::Value> = clusters
        .values()
        .map(|cluster| {
            json!({
                "cluster_id": cluster.cluster_id,
                "source": cluster.source,
                "representative_title": cluster.representative_title,
                "member_count": cluster.member_race_ids.len(),
                "last_updated": cluster.last_updated,
                "last_accessed": cluster.last_accessed,
                "stats": {
                    "mean": cluster.stats.mean,
                    "median": cluster.stats.median,
                    "std_dev": cluster.stats.std_dev,
                    "mad": cluster.stats.mad,
                    "sample_size": cluster.stats.recent_times.len(),
                    "percentiles": {
                        "p10": cluster.stats.percentiles.p10,
                        "p25": cluster.stats.percentiles.p25,
                        "p50": cluster.stats.percentiles.p50,
                        "p75": cluster.stats.percentiles.p75,
                        "p90": cluster.stats.percentiles.p90,
                        "p95": cluster.stats.percentiles.p95,
                    },
                    "trend": {
                        "direction": format!("{:?}", cluster.stats.trend.direction),
                        "rate": cluster.stats.trend.rate,
                        "confidence": cluster.stats.trend.confidence,
                    },
                    "eta_prediction": {
                        "expected_seconds": cluster.stats.calculate_eta().expected_seconds,
                        "confidence": cluster.stats.calculate_eta().confidence,
                        "lower_bound": cluster.stats.calculate_eta().lower_bound,
                        "upper_bound": cluster.stats.calculate_eta().upper_bound,
                    }
                }
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(cluster_list))
}

pub async fn get_cluster(
    path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    let cluster_id = path.into_inner();

    let clusters = data
        .prediction_engine
        .clustering_engine
        .clusters
        .read()
        .await;

    match clusters.get(&cluster_id) {
        Some(cluster) => {
            let detail = json!({
                "cluster_id": cluster.cluster_id,
                "source": cluster.source,
                "representative_title": cluster.representative_title,
                "representative_metadata": cluster.representative_metadata,
                "member_race_ids": cluster.member_race_ids,
                "member_count": cluster.member_race_ids.len(),
                "last_updated": cluster.last_updated,
                "last_accessed": cluster.last_accessed,
                "stats": {
                    "recent_times": cluster.stats.recent_times,
                    "mean": cluster.stats.mean,
                    "median": cluster.stats.median,
                    "std_dev": cluster.stats.std_dev,
                    "mad": cluster.stats.mad,
                    "sample_size": cluster.stats.recent_times.len(),
                    "percentiles": {
                        "p10": cluster.stats.percentiles.p10,
                        "p25": cluster.stats.percentiles.p25,
                        "p50": cluster.stats.percentiles.p50,
                        "p75": cluster.stats.percentiles.p75,
                        "p90": cluster.stats.percentiles.p90,
                        "p95": cluster.stats.percentiles.p95,
                    },
                    "trend": {
                        "direction": format!("{:?}", cluster.stats.trend.direction),
                        "rate": cluster.stats.trend.rate,
                        "confidence": cluster.stats.trend.confidence,
                    },
                    "eta_prediction": {
                        "expected_seconds": cluster.stats.calculate_eta().expected_seconds,
                        "confidence": cluster.stats.calculate_eta().confidence,
                        "lower_bound": cluster.stats.calculate_eta().lower_bound,
                        "upper_bound": cluster.stats.calculate_eta().upper_bound,
                    }
                }
            });
            Ok(HttpResponse::Ok().json(detail))
        }
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Cluster not found",
            "cluster_id": cluster_id
        }))),
    }
}

// ============ Historic Data Management ============

#[derive(Deserialize)]
pub struct HistoricRaceQuery {
    pub source: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
    pub include_events: Option<bool>,
    pub cursor: Option<String>,
}

pub async fn get_historic_races(
    query: web::Query<HistoricRaceQuery>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    // Preferred path: scan from persistence using time index
    let limit = query.limit.unwrap_or(100).min(10000); // Allow up to 10k records
    let filter = crate::persistence::RaceScanFilter {
        source: query.source.clone(),
        from: query.from,
        to: query.to,
        include_events: query.include_events.unwrap_or(false),
    };
    let batch = match data
        .persistence
        .scan_races(filter, limit, query.cursor.clone())
        .await
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to scan historic races from persistence: {}", e);
            // Transitional fallback: legacy JSON
            let mut path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            path.push(".raceboard");
            path.push("races.json");
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(mut races) = serde_json::from_str::<Vec<Race>>(&contents) {
                        // Naive filter + truncate for fallback
                        races.retain(|r| {
                            if let Some(ref src) = query.source {
                                if &r.source != src {
                                    return false;
                                }
                            }
                            if let Some(from) = query.from {
                                if r.started_at < from {
                                    return false;
                                }
                            }
                            if let Some(to) = query.to {
                                if r.started_at > to {
                                    return false;
                                }
                            }
                            true
                        });
                        races.sort_by(|a, b| a.started_at.cmp(&b.started_at));
                        races.truncate(limit);
                        return Ok(HttpResponse::Ok().json(json!({
                            "items": races,
                            "next_cursor": null,
                        })));
                    }
                }
            }
            return Ok(HttpResponse::InternalServerError().json(json!({
                "error": "internal",
                "message": "Failed to read historic races",
            })));
        }
    };

    // If empty and first page, try legacy JSON fallback once
    if batch.items.is_empty() && query.cursor.is_none() {
        let mut path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        path.push(".raceboard");
        path.push("races.json");
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(json_races) = serde_json::from_str::<Vec<Race>>(&contents) {
                    log::warn!(
                        "Historic: sled empty; returning {} races from JSON fallback",
                        json_races.len()
                    );
                    return Ok(HttpResponse::Ok().json(json!({
                        "items": json_races,
                        "next_cursor": null,
                        "total": json_races.len(),
                        "races": json_races,
                    })));
                }
            }
        }
    }

    // Backward-compatible shape: include legacy keys
    let total = batch.items.len();
    let items = batch.items;
    let items_clone = items.clone();
    Ok(HttpResponse::Ok().json(json!({
        "items": items,
        "next_cursor": batch.next_cursor,
        "total": total,
        "races": items_clone,
    })))
}

pub async fn delete_historic_races(
    query: web::Query<HistoricRaceQuery>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    // Historical deletion is not allowed via this endpoint; use admin purge.
    Ok(HttpResponse::MethodNotAllowed().json(json!({
        "error": "method_not_allowed",
        "message": "Historic deletion is disabled. Use admin purge API.",
        "query": {
            "source": query.source,
            "from": query.from,
            "to": query.to,
        }
    })))
}

// ============ Rebuild Metrics & Debug Info ============

pub async fn get_rebuild_metrics(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Get current rebuild metrics
    let rollout = data.rebuild_trigger.rollout_controller.read().await;
    let active_clusters = data.rebuild_clusters.active.read().await;
    let inactive_clusters = data.rebuild_clusters.inactive.read().await;

    // Calculate cluster statistics
    let cluster_stats = {
        let total_clusters = active_clusters.len();
        let sources: std::collections::HashSet<_> =
            active_clusters.values().map(|c| c.source.clone()).collect();
        let singleton_clusters = active_clusters
            .values()
            .filter(|c| c.member_race_ids.len() == 1)
            .count();
        let avg_cluster_size = if total_clusters > 0 {
            active_clusters
                .values()
                .map(|c| c.member_race_ids.len())
                .sum::<usize>() as f64
                / total_clusters as f64
        } else {
            0.0
        };

        json!({
            "total_clusters": total_clusters,
            "sources": sources,
            "singleton_clusters": singleton_clusters,
            "average_cluster_size": avg_cluster_size,
            "inactive_clusters": inactive_clusters.len(),
        })
    };

    // Get rollout metrics
    let rollout_metrics = json!({
        "current_phase": format!("{:?}", rollout.current_phase),
        "total_rebuilds": rollout.global_metrics.total_rebuilds,
        "successful_rebuilds": rollout.global_metrics.successful_rebuilds,
        "failed_rebuilds": rollout.global_metrics.failed_rebuilds,
        "success_rate": if rollout.global_metrics.total_rebuilds > 0 {
            rollout.global_metrics.successful_rebuilds as f64 /
            rollout.global_metrics.total_rebuilds as f64
        } else {
            0.0
        },
        "average_mae": rollout.global_metrics.average_mae,
        "average_noise_ratio": rollout.global_metrics.average_noise_ratio,
        "average_ari": rollout.global_metrics.average_ari,
        "rollback_count": rollout.global_metrics.rollback_count,
    });

    Ok(HttpResponse::Ok().json(json!({
        "cluster_stats": cluster_stats,
        "rollout_metrics": rollout_metrics,
        "buffer_status": {
            "active_clusters": active_clusters.len(),
            "inactive_clusters": inactive_clusters.len(),
        }
    })))
}

pub async fn get_rollout_status(data: web::Data<AppState>) -> Result<HttpResponse> {
    let rollout = data.rebuild_trigger.rollout_controller.read().await;

    // Build source status details
    let sources: Vec<_> = rollout
        .source_status
        .iter()
        .map(|(source, status)| {
            json!({
                "source": source,
                "enabled": status.enabled,
                "mode": format!("{:?}", status.mode),
                "last_rebuild": status.last_rebuild,
                "success_count": status.success_count,
                "failure_count": status.failure_count,
                "success_rate": if status.success_count + status.failure_count > 0 {
                    status.success_count as f64 /
                    (status.success_count + status.failure_count) as f64
                } else {
                    0.0
                },
                "parameters": {
                    "eps_range": status.current_parameters.eps_range,
                    "min_samples": status.current_parameters.min_samples,
                    "min_cluster_size": status.current_parameters.min_cluster_size,
                },
                "recent_validations": status.validation_results.iter()
                    .rev()
                    .take(5)
                    .map(|v| json!({
                        "passed": v.passed,
                        "mae": v.metrics.mae,
                        "noise_ratio": v.metrics.noise_ratio,
                        "ari": v.metrics.ari,
                        "failures": v.failures,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    // Get phase history
    let phase_history: Vec<_> = rollout
        .phase_history
        .iter()
        .rev()
        .take(10)
        .map(|transition| {
            json!({
                "from": format!("{:?}", transition.from_phase),
                "to": format!("{:?}", transition.to_phase),
                "timestamp": transition.timestamp,
                "reason": transition.reason,
                "metrics_at_transition": {
                    "total_rebuilds": transition.metrics_snapshot.total_rebuilds,
                    "success_rate": if transition.metrics_snapshot.total_rebuilds > 0 {
                        transition.metrics_snapshot.successful_rebuilds as f64 /
                        transition.metrics_snapshot.total_rebuilds as f64
                    } else {
                        0.0
                    },
                }
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(json!({
        "current_phase": format!("{:?}", rollout.current_phase),
        "sources": sources,
        "phase_history": phase_history,
        "config": {
            "pilot_source": rollout.config.pilot_source,
            "shadow_duration_hours": rollout.config.shadow_duration.num_hours(),
            "canary_duration_hours": rollout.config.canary_duration.num_hours(),
            "canary_percentage": rollout.config.canary_percentage,
            "success_threshold": rollout.config.success_threshold,
            "min_rebuilds_for_promotion": rollout.config.min_rebuilds_for_promotion,
            "auto_rollback": rollout.config.auto_rollback,
        }
    })))
}

#[derive(Deserialize)]
pub struct EnableAllSourcesPayload {
    /// One of: "shadow", "production", "canary"
    pub mode: Option<String>,
    /// Canary percentage when mode == canary
    pub percentage: Option<u8>,
}

pub async fn enable_all_sources(
    payload: Option<web::Json<EnableAllSourcesPayload>>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    // Default to Shadow if not specified
    let mode = match payload.as_ref().and_then(|p| p.mode.as_ref()) {
        Some(m) if m.eq_ignore_ascii_case("production") => RolloutMode::Production,
        Some(m) if m.eq_ignore_ascii_case("canary") => {
            let pct = payload.as_ref().and_then(|p| p.percentage).unwrap_or(10);
            RolloutMode::Canary { percentage: pct }
        }
        _ => RolloutMode::Shadow,
    };

    data.rebuild_trigger.enable_all_sources(mode).await;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "mode": match mode {
            RolloutMode::Shadow => "shadow",
            RolloutMode::Production => "production",
            RolloutMode::Canary { .. } => "canary",
            RolloutMode::Disabled => "disabled",
        },
    })))
}

// ====== Admin: Purge and Compaction ======

#[derive(Deserialize)]
pub struct PurgeRequest {
    pub race_ids: Vec<String>,
    pub reason: Option<String>,
    pub requested_by: Option<String>,
}

pub async fn admin_purge(
    body: web::Json<PurgeRequest>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    // Basic synchronous implementation: delete from persistence only; in-memory is for active races
    let req = body.into_inner();
    if req.race_ids.is_empty() {
        return Ok(HttpResponse::BadRequest().json(json!({
            "error": "invalid_request",
            "message": "race_ids must be a non-empty array",
        })));
    }
    use crate::persistence::RaceStore;
    let mut purged = Vec::new();
    let mut not_found = Vec::new();
    for id in req.race_ids.iter() {
        match data.persistence.delete_race(id).await {
            Ok(_) => purged.push(id.clone()),
            Err(_) => not_found.push(id.clone()),
        }
    }
    // Write audit record
    let audit = json!({
        "action": "purge",
        "requested_by": req.requested_by,
        "reason": req.reason,
        "timestamp": chrono::Utc::now(),
        "purged": purged,
        "not_found": not_found,
    });
    let _ = data.persistence.write_audit_record("purge", &audit);
    Ok(HttpResponse::Ok().json(json!({
        "purged": purged,
        "not_found": not_found,
    })))
}

#[derive(Serialize)]
struct AcceptedJob {
    status: &'static str,
    job_id: String,
}

pub async fn admin_compact(data: web::Data<AppState>) -> Result<HttpResponse> {
    if data.read_only {
        return Ok(HttpResponse::ServiceUnavailable()
            .insert_header(("X-Raceboard-Read-Only", "1"))
            .json(json!({"error":"read_only","message":"Server is in read-only mode"})));
    }
    // sled compaction is automatic; expose a no-op Accepted response for UX
    let job = AcceptedJob {
        status: "accepted",
        job_id: format!("compact_{}", chrono::Utc::now().timestamp()),
    };
    // Best effort flush
    if let Ok(size) = data.persistence.get_db_size() {
        log::info!("DB size before compaction hint: {} bytes", size);
    }
    // There's no public manual compaction trigger; calling flush to hint
    let _ = data.persistence.flush();
    Ok(HttpResponse::Accepted().json(job))
}

#[derive(serde::Serialize)]
struct StorageReport {
    schema_version: Option<String>,
    races_count: usize,
    index_entries: usize,
}

pub async fn admin_storage_report(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Count races by reading persistence layer
    let version = data.persistence.get_schema_version();
    let races_count = data.persistence.races_count();
    let index_entries = data.persistence.index_entries();
    Ok(HttpResponse::Ok().json(StorageReport {
        schema_version: version,
        races_count,
        index_entries,
    }))
}

pub async fn admin_metrics(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Get comprehensive metrics from the data layer
    if let Some(ref metrics) = data.data_layer_metrics {
        let summary = metrics.get_metrics_summary().await;
        let slo_violations = metrics.check_slos().await;

        Ok(HttpResponse::Ok().json(json!({
            "metrics": summary,
            "slo_violations": slo_violations,
            "timestamp": chrono::Utc::now(),
        })))
    } else {
        Ok(HttpResponse::ServiceUnavailable().json(json!({
            "error": "metrics_unavailable",
            "message": "Data layer metrics not initialized"
        })))
    }
}

pub async fn trigger_rebuild(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Manually trigger a rebuild
    match data.rebuild_trigger.trigger_rebuild().await {
        Ok(_) => Ok(HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Rebuild triggered successfully"
        }))),
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Failed to trigger rebuild: {}", e)
        }))),
    }
}

pub async fn reset_rollout_phase(data: web::Data<AppState>) -> Result<HttpResponse> {
    // Reset rollout to Phase 1 (single source)
    data.rebuild_trigger.reset_to_phase_1().await;

    Ok(HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Rollout reset to Phase 1 (single source)",
        "phase": "SingleSource",
        "pilot_source": "cargo"
    })))
}

pub async fn get_cluster_debug(
    path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    let cluster_id = path.into_inner();

    let clusters = data.rebuild_clusters.active.read().await;

    match clusters.get(&cluster_id) {
        Some(cluster) => {
            // Get member races for detailed debug info
            let all_races = data.storage.get_all_races().await;
            let member_races: Vec<_> = all_races
                .iter()
                .filter(|r| cluster.member_race_ids.contains(&r.id))
                .map(|r| {
                    json!({
                        "id": r.id,
                        "title": r.title,
                        "state": r.state,
                        "started_at": r.started_at,
                        "eta_sec": r.eta_sec,
                        "metadata": r.metadata,
                    })
                })
                .collect();

            let debug_info = json!({
                "cluster_id": cluster.cluster_id,
                "source": cluster.source,
                "member_count": cluster.member_race_ids.len(),
                "representative_title": cluster.representative_title,
                "member_titles": cluster.member_titles,
                "member_races": member_races,
                "stats": {
                    "recent_times": cluster.stats.recent_times,
                    "mean": cluster.stats.mean,
                    "median": cluster.stats.median,
                    "std_dev": cluster.stats.std_dev,
                    "mad": cluster.stats.mad,
                    "trend": {
                        "direction": format!("{:?}", cluster.stats.trend.direction),
                        "rate": cluster.stats.trend.rate,
                        "confidence": cluster.stats.trend.confidence,
                    }
                },
                "metadata_history": cluster.member_metadata_history,
                "last_updated": cluster.last_updated,
                "last_accessed": cluster.last_accessed,
            });

            Ok(HttpResponse::Ok().json(debug_info))
        }
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Cluster not found",
            "cluster_id": cluster_id
        }))),
    }
}

// ============================================================================
// Adapter Status Handlers
// ============================================================================

use crate::adapter_status::{AdapterRegistration, AdapterMetrics};

/// Register an adapter (self-registration)
pub async fn adapter_register(
    registration: web::Json<AdapterRegistration>,
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    let registration = registration.into_inner();
    
    // Register the adapter
    data.adapter_registry.register(registration.clone()).await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;
    
    log::info!(
        "Adapter registered: {} (instance: {})",
        registration.adapter_type.as_str(),
        registration.instance_id
    );
    
    Ok(HttpResponse::Ok().json(json!({
        "status": "registered",
        "adapter_id": registration.id,
        "health_interval_seconds": registration.health_interval_seconds,
        "message": "Adapter successfully registered"
    })))
}

/// Report adapter health
pub async fn adapter_health(
    data: web::Data<AppState>,
    req: web::Json<serde_json::Value>,
) -> Result<HttpResponse> {
    let adapter_id = req.get("adapter_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| actix_web::error::ErrorBadRequest("Missing adapter_id"))?;
    
    let metrics = req.get("metrics")
        .and_then(|v| serde_json::from_value::<AdapterMetrics>(v.clone()).ok())
        .unwrap_or_default();
    
    let error = req.get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    
    // Update health
    data.adapter_registry.report_health(adapter_id, metrics, error).await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;
    
    Ok(HttpResponse::Ok().json(json!({
        "status": "ok",
        "adapter_id": adapter_id,
        "message": "Health report received"
    })))
}

/// Deregister an adapter (clean shutdown)
pub async fn adapter_deregister(
    data: web::Data<AppState>,
    req: web::Json<serde_json::Value>,
) -> Result<HttpResponse> {
    let adapter_id = req.get("adapter_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| actix_web::error::ErrorBadRequest("Missing adapter_id"))?;
    
    // Deregister
    data.adapter_registry.deregister(adapter_id).await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;
    
    log::info!("Adapter deregistered: {}", adapter_id);
    
    Ok(HttpResponse::Ok().json(json!({
        "status": "deregistered",
        "adapter_id": adapter_id,
        "message": "Adapter successfully deregistered"
    })))
}

/// Get adapter status for UI
pub async fn get_adapter_status(
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    let adapters = data.adapter_registry.get_all().await;
    let summary = data.adapter_registry.get_summary().await;
    
    let mut adapter_list = Vec::new();
    for (reg, health) in adapters {
        adapter_list.push(json!({
            "id": reg.id,
            "type": reg.adapter_type.as_str(),
            "instance": reg.instance_id,
            "display_name": reg.display_name,
            "version": reg.version,
            "state": health.state.as_str(),
            "state_color": health.state.color(),
            "registered_at": reg.registered_at,
            "last_report": health.last_report,
            "seconds_since_report": health.seconds_since_report(),
            "metrics": {
                "races_created": health.metrics.races_created,
                "races_updated": health.metrics.races_updated,
                "last_activity": health.metrics.last_activity,
                "error_count": health.metrics.error_count,
            },
            "error": health.error,
            "pid": reg.pid,
        }));
    }
    
    Ok(HttpResponse::Ok().json(json!({
        "adapters": adapter_list,
        "summary": {
            "total": summary.total_adapters,
            "healthy": summary.healthy_count(),
            "unhealthy": summary.unhealthy_count(),
            "operational": summary.all_operational(),
            "races_created": summary.total_races_created,
            "races_updated": summary.total_races_updated,
            "last_update": summary.last_update,
        }
    })))
}

/// Get Prometheus metrics
pub async fn get_adapter_metrics(
    data: web::Data<AppState>,
) -> Result<HttpResponse> {
    let metrics = data.adapter_registry.export_metrics().await;
    
    Ok(HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(metrics))
}
