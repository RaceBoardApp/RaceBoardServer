# Raceboard Architecture Assessment and Critique (2025-09-11)

This document provides an evidence-based assessment of the Raceboard Server architecture based on the codebase and project documentation under `docs/`.

Scope:
- Server runtime (HTTP and gRPC)
- In-memory storage and persistence
- Prediction/clustering and rebuild flow
- Adapter model and health subsystem
- API surfaces and data model
- Operational characteristics (observability, reliability, scalability)

References:
- ../guides/SERVER_GUIDE.md
- ../specs/DATA_LAYER_SPECIFICATION.md (v7)
- grpc/race.proto
- src/main.rs, src/app_state.rs, src/handlers.rs, src/grpc_service.rs, src/persistence.rs, src/storage.rs, src/adapter_status.rs, src/models.rs

Protocol Strategy Update (2025-09-10):
- gRPC is designated for UI/ops read-only access and streaming (UI only).
- REST is designated for adapter ingestion, mutations, and adapter lifecycle/health (adapters only).
- The existing gRPC adapter RPCs are deprecated and will be removed after a migration window. This review reflects that direction.

Review Update (2025-09-11):
- gRPC no longer infers adapter health or deregisters adapters based on race updates; adapter lifecycle is handled exclusively via REST endpoints and AdapterRegistry.
- gRPC CreateRace rejects adapter:* IDs and ListRaces filters adapter registrations; SystemStatus now derives adapter counts purely from AdapterRegistry.
- Adapters have been refactored to use REST-only registration/health via AdapterHealthMonitor; REST race handlers now reject adapter:* IDs and no longer infer registration/health from race metadata.
- [Fixed 2025-09-11] gRPC read-only enforcement implemented: CreateRace, UpdateRace, AddEvent, and DeleteRace now reject writes when server.read_only=true.
- [Fixed 2025-09-11] gRPC eta_history mapping completed in both directions (race_to_proto and proto_to_race).
- [Fixed 2025-09-11] Legacy JSON fallback for historical reads/writes is now gated behind configuration (`server.legacy_json_fallback_enabled`). Default: enabled for compatibility. Plan: disable by default in next release and remove after deprecation window.
- [Added 2025-09-11] Canonical adapter ID validator introduced; REST and gRPC use it to consistently reject adapter:* IDs on race endpoints.
- [Added 2025-09-11] Centralized ETA inference: helpers in models (infer_eta_source/confidence/update_interval_hint) used by REST and gRPC to avoid drift.
- [Fixed 2025-09-11] Logging hygiene: downgraded hot-path diagnostic logs to debug/trace; reserved warn/error for anomalies; added race_id fields to key logs.

## 1) Architectural Overview

High-level components:
- HTTP (Actix-web) and gRPC (Tonic) servers share a single runtime.
- Hot Path: Active races are kept in-memory in `Storage` with a broadcast channel to stream updates to gRPC clients.
- Historical Store: Completed races are persisted in sled using a time-index (`races_by_time`) for efficient historical scans.
- Prediction System: `PredictionEngine` uses a `ClusteringEngine` to compute ETAs; rebuild workflows double-buffer clusters and control rollout phases.
- Adapter Subsystem: Adapters exist as separate binaries/processes; the server maintains an `AdapterRegistry` (push-based health/state machine) and exposes REST APIs only for adapter registration and health (gRPC adapter RPCs deprecated).
- Monitoring/Alerts: `MonitoringSystem` and `AlertSystem` provide health/metrics and optional alerting; admin/diagnostic HTTP endpoints expose health and metrics.

Data flow (typical):
1. Adapter submits new/updated race via HTTP only (`POST /race`, `PATCH /race/{id}`).
2. `Storage` upserts in-memory race and emits a `StorageEvent` via broadcast.
3. gRPC `StreamRaces` maps storage events into streaming `RaceUpdate` messages for UIs.
4. When a race completes (Passed/Failed/Canceled), the server persists it to sled and updates indices. Historical REST queries use the sled time index.
5. Prediction engine updates ETA for new/updated races if not provided by adapters, using cluster statistics.
6. AdapterRegistry tracks adapter lifecycle and health; admin endpoints and gRPC provide status views and cleanup.

Key design choices:
- Two-plane model (hot path vs historical) – reduces latency for UI streaming and decouples persistence.
- Sled for embedded persistence with explicit time index and JSON serialization (legacy bincode fallback).
- Push-based adapter health with strict state machine and Prometheus export.


## 2) Module Responsibilities and Boundaries

- src/storage.rs: In-memory map of races + event broadcasting; capacity limits and per-race event caps.
- src/persistence.rs: Sled-backed data layer: races (completed), clusters, source stats, meta. Provides time-indexed scans; JSON-first serialization with bincode fallback; snapshot/export and audit helpers.
- src/grpc_service.rs: gRPC service implementation: streaming updates, CRUD RPCs for races, and adapter management RPCs.
- src/handlers.rs: HTTP REST handlers: races CRUD, clusters views, historic scans, admin endpoints (purge/compact/storage-report/metrics), rollout/rebuild controls, adapter REST endpoints.
- src/adapter_status.rs: Adapter registration and health state machine, metrics export, periodic monitoring.
- src/app_state.rs: DI container holding pointers to Storage, Prediction, Persistence, Rebuild, Monitoring, AdapterRegistry, etc.
- grpc/race.proto: Source of truth for gRPC surface, including Race, events, and adapter status messages.

Observations about coupling:
- Handlers and gRPC service both know about adapter registry and persistence; business rules (e.g., when to persist, how to register adapters) are duplicated.
- ETA inference hints exist in both models.rs and handlers.rs, leading to potential drift.
- Previously, the gRPC service interpreted adapter health from general race updates (metadata), which coupled unrelated concerns. This has been removed as of 2025-09-11. REST race handlers now also reject adapter:* IDs and no longer infer registration/health from race metadata, eliminating that coupling in favor of explicit REST adapter endpoints.


## 3) Data Model and Persistence

- Race model adds optimistic progress fields and ETA history; ETA revisions tracked in memory and serialized to JSON when persisted.
- Persistence uses dedicated trees and a time-ordered secondary index (`races_by_time`), enabling windowed scans with cursor encoding (base64 JSON with sec/nanos/id).
- Serialization: JSON-first to resolve bincode issues with complex types; fallback to bincode maintains backward compatibility.
- Snapshots: compressed JSON snapshot + SHA-256 checksum; retention cleanup; audit records recorded in `meta`.

Strengths:
- Clear separation between hot-path memory and historical persistence.
- Time index and cursor-based scans scale better than full-table scans.
- Robust, debuggable JSON storage; migration compatibility addressed.

Gaps/Risks:
- No explicit write-ahead or transactional grouping across trees (races + index update are best-effort). Crash between write and index update could create inconsistencies; code partially compensates but lacks verification tooling. 
- Persistence flush strategy (batching/latency targets) is described in docs but not fully enforced in code (e.g., 250 ms batching window not implemented; operations call `flush()` frequently on hot paths).
- Legacy JSON fallback file in `~/.raceboard/races.json` remains in handlers, creating dual sources of truth and operational ambiguity. We need remove falllback write, but keeps backups to json.


## 4) Adapter Model and Health

- Adapters register and report health via REST endpoints only. The previously added gRPC adapter RPCs (`RegisterAdapter`, `ReportAdapterHealth`, `DeregisterAdapter`) are deprecated and slated for removal.
- Adapter health state machine is well-defined with thresholds and periodic checks; Prometheus metrics provided.

Strengths:
- Clear, typed health model; background monitoring keeps states fresh; cleanup of stale entries.

Gaps/Risks:
- [Resolved 2025-09-11] Dual registration/update paths (REST vs gRPC) previously risked inconsistent behavior. REST race handlers now reject `adapter:` IDs and no longer infer health/registration from metadata; gRPC no longer infers adapter health or deregisters adapters. Adapters must use REST-only lifecycle endpoints.
- Security/auth not implemented on server endpoints. Current deployment assumption: all components run locally on a trusted host/network, and ports bind to localhost. This is acceptable for local-only setups but would be insufficient for multi-host or exposed environments. See Security Posture below for guidance.
- Adapter identity is derived from ID format strings; no canonical ID validation helper; repeated parsing logic.

### Security Posture (2025-09-11)
- Assumption: Single-host, local-only deployment. Adapters and server run on the same machine and communicate over 127.0.0.1.
- Implication: No authn/authz is enforced by the server. Any local process can call REST adapter endpoints to register/report health.
- Risk (if exposed): If HTTP/gRPC ports are exposed beyond localhost, unauthenticated writes become possible. Do not expose directly without protections.
- Recommendations (if/when needed):
  - Keep bindings on 127.0.0.1 and front with a reverse proxy (e.g., Nginx, Caddy, Traefik) enforcing auth (basic/OIDC) and IP allow-lists.
  - Consider mTLS between trusted nodes for multi-host deployments, or signed bearer tokens for adapters.
  - Lock down admin endpoints if ever exposed (auth required; rate-limits).
  - Introduce an optional feature-flagged auth layer before widening deployment scope.

## 5) API Surfaces and Contract Fidelity

- gRPC eta_history mapping is implemented; protobuf and internal model are in sync as of 2025-09-11.
- Read-only mode is enforced across HTTP and gRPC; mutating gRPC RPCs reject writes when server.read_only=true.

Gaps/Risks:
- [Resolved 2025-09-11] Previously, divergence between proto and server mapping caused silent data loss of ETA history over gRPC.
- [Resolved 2025-09-11] Read-only enforcement is now consistent across HTTP and gRPC.


## 6) Operational Characteristics

- Logging: many `warn!` and `error!` calls in hot paths (storage insert, persistence store/scan) may be noisy in production.
- Capacity/Eviction: Storage evicts oldest race when capacity is reached; this is logged but not measurable via metrics exposed by the server (outside of MonitoringSystem). Evictions directly impact clustering quality.
- Observability: Adapter metrics exported in Prometheus format; data-layer metrics exist but not fully integrated everywhere; server resource metrics (CPU/mem) are TODOs in gRPC status.


## 7) Strengths Summary

- Two-plane model is appropriate for latency-sensitive UI streaming.
- Sled layout and time index offer good balance for embedded persistence.
- Health state machine for adapters is explicit and operable (with cleanup and metrics).
- Clustering/rebuild double-buffer pattern with rollout phases is a sound approach to reduce blast radius.


## 8) Issues and Architectural Smells

- Duplication of business logic across HTTP/gRPC and in multiple modules (ETA inference, adapter registration/health handling, persistence triggers).
- Mixed responsibilities in REST handlers (including file system JSON fallbacks, which leak legacy behavior into current runtime).
- Hot path logging at warn/error levels that appear diagnostic rather than exceptional.
- Incomplete adherence to data layer spec (batching/flush policy not enforced programmatically; transactional grouping absent).
- Incomplete proto-to-model mapping (e.g., ETA history not handled) and inconsistent read-only enforcement across protocols.


## 9) Recommendations (Prioritized, Low-Risk First)

P0 — Correctness/Contract fidelity:
- Complete gRPC mapping for `eta_history` between internal `models::Race` and `race.proto` (both directions).
- Enforce read-only mode in gRPC mutating RPCs (CreateRace, UpdateRace, AddEvent, DeleteRace).

P0 — Source of truth consistency:
- Remove legacy `~/.raceboard/races.json` fallback and writes from handlers once sled is validated in staging. Gate behind a feature flag or configuration for a controlled deprecation period.

P1 — API simplification and boundary hardening:
- Enforce REST-only adapter lifecycle: use REST for Register/Health/Deregister; deprecate and remove gRPC adapter RPCs. Add validation helpers for adapter IDs and a single adapter ID parser.
- Centralize ETA inference rules inside `models` or a dedicated `eta` module; call from both HTTP and gRPC paths to avoid drift.

P1 — Reliability and data integrity:
- Introduce a minimal transactional write helper for persistence that writes race + time index atomically, or implement a compensating repair on startup to rebuild the time index from `races` when inconsistencies detected.
- Implement configurable flush batching (e.g., buffered channel + timer) to meet the documented RPO target without excessive `flush()` calls.

P2 — Observability and ops:
- Downgrade hot-path diagnostic logs to `debug!`/`trace!`, reserve `warn!`/`error!` for true anomalies; consider a structured logging field for `race_id`.
- Expose a metric for storage evictions and `max_races` utilization; alert when above thresholds.
- Populate system metrics (CPU/mem/uptime) in gRPC SystemStatus.

P2 — Developer experience:
- Extract shared interfaces for Storage and Persistence (traits) and inject via `AppState` to enable test doubles; reduce direct cross-module coupling and make behavior consistent across HTTP/gRPC.
- Add integration tests that exercise: adapter registration via gRPC, race life cycle, persistence on completion, and historic scan with cursors.

P3 — Security (as applicable in your environment):
- Add authn/authz on admin endpoints and adapter RPCs (mTLS or token-based auth); enforce source allow-lists for adapter registration.


## 10) Minimal Changes Suggested for Near-Term

The following concrete, low-risk improvements provide immediate value with minimal code churn:
- Add a link to this review in docs/index.md for discoverability.
- Document deprecation plan for legacy JSON fallback and dual adapter registration paths.
- Implement gRPC read-only checks and eta_history field mapping as discrete, local changes.

These are intentionally scoped to avoid broad refactors while improving correctness and operator clarity.


## 11) Appendix: Noted Code Targets

- gRPC read-only checks: `src/grpc_service.rs` in `create_race`, `update_race`, `add_event`, and `delete_race`.
- ETA history mapping: `src/grpc_service.rs` in `race_to_proto` and `proto_to_race` (implemented).
- Legacy fallback removal points: `src/handlers.rs` `get_historic_races` fallback branch and JSON writes in `update_race` completion block.
- Adapter registration: `src/handlers.rs` `create_race` implicit registration block; consider feature gating.
