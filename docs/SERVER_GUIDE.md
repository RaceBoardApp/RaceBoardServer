# Raceboard Server Guide

This document describes how to build, run, configure, and integrate with the Raceboard server. It consolidates scattered notes into a single practical guide.

## Overview
- Language: Rust (Actix-web for HTTP, Tonic for gRPC)
- Binaries:
  - `raceboard-server`: HTTP + gRPC server
  - other adapters/tools live under `src/bin/`
- APIs:
  - HTTP: see `api/openapi.yaml`
  - gRPC: see `grpc/race.proto`
- Persistence: sled (embedded key–value store) in `~/.raceboard/eta_history.db` by default

## Build & Test
- Build: `cargo build`
- Run server: `cargo run --bin raceboard-server`
- Tests: `cargo test`
- Lint/format: `cargo clippy -- -D warnings` and `cargo fmt`

## Run
The server exposes:
- HTTP (default `http://localhost:7777`)
  - `/health` — health probe
  - `/races` — list races (GET)
  - `/race` — create a race (POST)
  - `/race/{id}` — update a race (PATCH)
  - `/race/{id}` — delete a race (DELETE)
- gRPC (default on 50051) — streaming updates for UI clients

Start locally:
```
cargo run --bin raceboard-server
```
Sanity check:
```
curl http://localhost:7777/health
```

## Configuration
Configuration is read from `config.toml` in the repo root, then overridden by environment variables. See also `docs/CONFIGURATION.md` for full details.

Example `config.toml` (excerpt):
```
[server]
http_port = 7777
grpc_port = 50051

[logging]
level = "info"
```

Environment overrides use a double underscore to separate table keys, for example:
```
RACEBOARD_SERVER__HTTP_PORT=8080 \
RACEBOARD_LOGGING__LEVEL=debug \
cargo run --bin raceboard-server
```

## Persistence
By default the server stores data in sled at `~/.raceboard/eta_history.db`.
- The tests use an in‑memory sled instance.
- If the database is locked (another instance running), startup prints a clear error.

## Read‑Only Mode
During maintenance operations, the server may enter a read‑only mode. Clients should expect:
- HTTP 503 with header `X-Raceboard-Read-Only: 1` on write endpoints.
- Adapters should retry later (the calendar adapter already does this).

## Security & Local Deployment
- Current assumption: All components (server and adapters) run on the same machine and communicate over localhost (127.0.0.1). No authentication is implemented on HTTP/gRPC endpoints.
- Do NOT expose the server directly to untrusted networks. If you must, front it with a reverse proxy that enforces authentication and IP allow-lists.
- Recommended if exposing beyond localhost:
  - Keep server bindings on 127.0.0.1 and use a reverse proxy (Nginx/Caddy/Traefik) to terminate TLS and enforce auth (Basic/OIDC) and network policies.
  - Consider mTLS or signed tokens for adapter-to-server calls in multi-host deployments.
  - Protect admin endpoints (/admin/*, /metrics/*) with auth and rate limits.

## API Surfaces
- Protocol strategy: UI clients use gRPC (read-only, streaming); Adapters use REST (writes and health).
- HTTP: `api/openapi.yaml` is the source of truth for request/response shapes.
- gRPC: `grpc/race.proto` defines streaming update messages for UI clients and is read-only for UI/ops. Adapter-oriented gRPC RPCs are deprecated.

### Additional HTTP Endpoints
These are primarily diagnostics/admin surfaces exposed by the server:
- Metrics and rollout:
  - `GET /metrics/rebuild` — rebuild/cluster metrics
  - `GET /metrics/rollout` — phased rollout status
- Clusters:
  - `GET /clusters` — list ETA clusters (summary)
  - `GET /cluster/{id}` — detailed cluster view
- Historic data (completed races persisted in sled):
  - `GET /historic/races` — time-ordered scan with filters (`source`, `from`, `to`, `limit`, `include_events`, `cursor`)
- Admin:
  - `POST /admin/purge` — purge transient data (use cautiously)
  - `POST /admin/compact` — compact/flush
  - `GET /admin/storage-report` — basic persistence stats
  - `GET /admin/metrics` — data layer metrics summary

## Logging
The server uses `log` + `env_logger`. Set `RUST_LOG` to control verbosity, e.g.:
```
RUST_LOG=info cargo run --bin raceboard-server
RUST_LOG=debug,hyper=warn,tower=warn cargo run --bin raceboard-server
```

## Adapters
Adapters are independent binaries that POST to the server HTTP API. Adapters must use REST for all writes and health reporting; gRPC is reserved for UI/ops and adapter-oriented gRPC RPCs are deprecated. Note: Race endpoints (/race, /race/{id}, /race/{id}/event) explicitly reject adapter:* IDs; adapters must use /adapter/register, /adapter/health, and /adapter/deregister for lifecycle and health.

Selected docs:
- Codex log watcher: `src/bin/raceboard_codex_watch.rs` (see `docs/CODEX_LOG_TRACKING.md`)
- Google/ICS calendar free‑time: `src/bin/raceboard_calendar.rs` (see `docs/GOOGLE_CALENDAR_ADAPTER.md`)
- Shell runner and others: see `docs/ADAPTER_DEVELOPMENT_GUIDE.md`

## Development Tips
- Rebuild after changing protobufs in `grpc/race.proto`: `cargo build`
- Avoid printing with `println!` in runtime code; use `log` macros
- Keep modules small and testable; prefer `#[tokio::test]` for async tests
