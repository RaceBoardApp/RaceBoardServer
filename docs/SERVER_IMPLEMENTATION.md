# Server Implementation

Related docs:
- Server Guide: `docs/SERVER_GUIDE.md`
- Configuration: `docs/CONFIGURATION.md`
- Data layer and persistence: `docs/DATA_LAYER_SPECIFICATION.md`
- ETA prediction system: `docs/ETA_PREDICTION_SYSTEM.md`
- HTTP API (OpenAPI): `api/openapi.yaml` and gRPC: `grpc/race.proto`

This document describes the current implementation of the Raceboard server, based on the source code.

## Overview

The server is a Rust application that serves as the central hub for the Raceboard system. It provides a gRPC API for the UI and a REST API for the adapters.

## Core Components

### `main.rs`

- Loads settings from `config.toml` and env overrides.
- Spawns two services:
    - A gRPC server using `tonic`.
    - An HTTP server using `actix-web`.
- Initializes in‑memory `Storage`, the `PredictionEngine` (with clustering), the sled‑backed `PersistenceLayer`, and the rebuild/monitoring subsystems.

### `models.rs`

- Defines the core data structures: `Race`, `RaceState`, `RaceUpdate`, and `Event`.
- `Race` fields (as serialized over HTTP):
    - `id: string`
    - `source: string`
    - `title: string`
    - `state: "queued"|"running"|"passed"|"failed"|"canceled"`
    - `started_at: RFC3339 date-time`
    - `completed_at?: RFC3339 date-time` (set by server when terminal state reached)
    - `duration_sec?: integer` (server-calculated on completion)
    - `eta_sec?: integer`
    - `progress?: integer`
    - `deeplink?: string`
    - `metadata?: map<string,string>`
    - `events?: Event[]` (optional for hot path and historic scans)

### `storage.rs`

- Implements the hot‑path, in‑memory `Storage` for active races.
- Uses a `RwLock<HashMap<String, Race>>` and a broadcast channel for change events.
- Provides methods for create/update/delete and event append, with limits (`max_races`, `max_events_per_race`).
- Does not persist to disk on change; persistence of completed races is handled separately.

### `persistence.rs`

- Sled‑backed `PersistenceLayer` for historical/completed races and cluster/state data.
- Stores races in JSON (with legacy bincode fallback), and maintains a time index for efficient scans.
- Location by default: `~/.raceboard/eta_history.db`.
- Also persists ETA clusters and per‑source statistics; includes migration helpers and daily JSON snapshots.

### `grpc_service.rs`

- Implements the `RaceService` gRPC service.
- Provides the following RPCs:
    - `ListRaces`: Returns a list of all current races.
    - `StreamRaces`: Streams real-time updates for all races.
    - `GetRace`: Returns a single race by its ID.
    - `CreateRace`: Creates a new race.
    - `UpdateRace`: Updates an existing race.
    - `DeleteRace`: Deletes a race by its ID.

### `handlers.rs`

- Implements the REST API using `actix-web`.
- Core endpoints:
    - `GET /health`
    - `GET /races`
    - `POST /race`
    - `GET /race/{id}`
    - `PATCH /race/{id}`
    - `DELETE /race/{id}`
- Additional endpoints (diagnostics/admin): clusters (`/clusters`, `/cluster/{id}`), rebuild metrics (`/metrics/rebuild`), rollout (`/metrics/rollout`, `/rollout/enable_all`, `/rollout/reset`), admin (`/admin/*`), historic scans (`/historic/races`).
