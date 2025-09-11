# Data Layer Specification (v7)

Related docs:
- Server Guide: `../guides/SERVER_GUIDE.md`
- ETA prediction system: `../design/ETA_PREDICTION_SYSTEM.md`
- Server implementation notes: `../design/SERVER_IMPLEMENTATION.md`

This document is the implementation‑grade specification for Raceboard's data layer. It is designed to be clean, robust, and complete while storing only what is necessary for ETA predictions, UI, and rebuild workflows.

**Implementation Note (2025-09-02)**: The Envelope serialization pattern described in the original specification has been replaced with direct JSON serialization due to compatibility issues with bincode and complex Rust types. See Section 3 for the current implementation approach.

Two‑plane model:
- Hot Path (UI/gRPC): active races live in memory only and are the sole source for gRPC methods and UI views.
- Historical Store (Persistence): completed races are persisted for clustering, rebuilds, analytics, and historic REST queries; the UI does not read persisted races.

## 1. Source of Truth & Durability

- Source of Truth: `sled` is the single source of truth. JSON snapshots are strictly for disaster recovery and manual inspection.
- Durability Goals: target RPO ≤ 250 ms (bounded by flush window) and RTO < 5 minutes.
- Flush Policy: persist and flush in batches of up to 100 writes or 250 ms, whichever comes first; force flush on shutdown. Publish flush latency metrics. Note: true RPO=0 under sudden power loss depends on OS/page cache; the bounded unflushed window is ≤ 250 ms by policy.

## 2. Single‑Instance & Read‑Only Mode

- Exclusive Lock: if the sled lock cannot be acquired, the server exits by default.
- Read‑Only Mode: enable with `--read-only` or `RACEBOARD_READ_ONLY=1`. In read‑only, all mutating endpoints return 503 with a diagnostic header; exit requires restart.

## 3. Storage Layout & Envelope

- Trees:
  - `races`: historical race records (completed only; not used by gRPC/UI).
  - `clusters`: clustering state per `cluster_id`.
  - `source_stats`: aggregate statistics per `source`.
  - `meta`: schema versioning, migration reports, audits, idempotency tokens.
- Indexes (for efficient scans):
  - `races_by_time`: key = `<started_at_be><race_id>`; value = empty. Used for time‑range streaming without full scans.
- Keys:
  - `races`: `<race_id>` (string UUID or adapter‑provided id).
  - `clusters`: `<cluster_id>`.
  - `source_stats`: `<source>`.
  - `meta/*`: namespaced keys (e.g., `schema_version`, `migrations/<ts>`, `audit/<uuid>`, `idempotency/<token>`).
- Serialization Format:
  - **Current Implementation**: Direct JSON serialization without envelope wrapper
  - **Reason**: Bincode serialization with Envelope pattern caused compatibility issues with PhantomData and complex nested structures
  - **Format**: All values are stored as JSON-serialized bytes directly
  - **Backward Compatibility**: Deserializer attempts JSON first, falls back to bincode for legacy data
  - **Schema Version**: Tracked in `meta/schema_version` key (currently version 2)

Implementation Note:
```rust
// Simplified serialization approach (actual implementation)
fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
}

fn deserialize<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    // Try JSON first, then bincode for backward compatibility
    serde_json::from_slice(data)
        .or_else(|_| bincode::deserialize(data))
}
```

This approach provides:
- Simpler implementation without nested envelope complexity
- Better debugging (human-readable JSON in database)
- Compatibility with all Rust types including PhantomData
- Easy migration path from legacy bincode data

## 4. Data Kept (Minimal but Sufficient)

- Race (persisted on completion only): `id, source, title, state, started_at, eta_sec?, progress?, deeplink?, metadata?, events?` with event cap.
  - Event Retention: `max_events_per_race` enforced; truncate oldest first. Persist derived duration explicitly on completion to avoid reliance on full event history. Active race state for UI remains in memory.
- Cluster: `cluster_id, source, representative_title, representative_metadata (centroid), stats, member_race_ids (bounded ≤100), member_titles (≤50), member_metadata_history (≤50), last_updated, last_accessed`.
- SourceStats: per source rolling `execution_history` (bounded) + aggregate `stats`, timestamps, and `max_history_size`.

## 5. Write Path & Consistency

- Consistency Model: eventually consistent; reads may observe up to 5 seconds of staleness under retry.
- Write Order (race completion): 1) persist completed race (historical store), 2) update source_stats, 3) update clusters.
  - On creation/update before completion: write to in‑memory store only (UI/gRPC); persistence is not on the critical path.
- Idempotency:
  - Key: `<race_id>:<event_seq>` or `<race_id>:completion` stored in `meta/idempotency/<token>` with TTL=24h.
  - Behavior: duplicate tokens short‑circuit as success; handlers are idempotent.
- Retries:
  - Backoff with jitter; max 5 attempts per stage; failed stages enqueue for background retry; DLQ is logged and surfaced via metrics; compensations are not needed due to idempotency and monotonic updates.

Persistence signatures (Rust):

```
#[async_trait]
trait Persistence {
    async fn store_race(&self, race: &Race, idem: Option<&str>) -> Result<()>;
    async fn get_race(&self, id: &str) -> Result<Option<Race>>;
    async fn scan_races(&self, filter: RaceScanFilter, batch_size: usize, cursor: Option<String>) -> Result<RaceBatch>;
    fn upsert_cluster(&self, cluster: &RaceCluster) -> Result<()>;
    fn load_clusters(&self) -> Result<HashMap<String, RaceCluster>>;
    fn persist_source_stats(&self, source: &str, stats: &SourceStats) -> Result<()>;
    fn load_source_stats(&self) -> Result<HashMap<String, SourceStats>>;
}

struct RaceScanFilter { source: Option<String>, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, include_events: bool }
struct RaceBatch { items: Vec<Race>, next_cursor: Option<String> }
```

Concrete types (Rust):

```
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaceScanFilter {
    pub source: Option<String>,
    pub from:   Option<DateTime<Utc>>,
    pub to:     Option<DateTime<Utc>>,
    pub include_events: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaceBatch<T = crate::models::Race> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}
```

Error semantics:
- Cross-tree atomicity is not guaranteed. Each stage is idempotent; failures at any stage are retried with backoff.
- On partial success, later reads may observe earlier stages until retries complete.
- All write APIs return typed errors; permanent errors are surfaced via logs/metrics and, for admin APIs, as structured responses.

## 6. Read Path & Startup

- gRPC/UI reads: served exclusively from the in‑memory active store; persistence is never read on the UI path.
- On startup: load `clusters` and `source_stats`. Do not warm persisted races into memory (historical store is for rebuilds/analytics only).
- Historic queries: served from `races` tree with pagination and filters (see §8).

Index maintenance rules:
- On race create: insert into `races` and add an entry to `races_by_time` with key = `started_at` big‑endian bytes concatenated with `race_id` (to ensure lexicographic ordering), value empty.
- On race update:
  - If `started_at` changes, remove old `races_by_time` key and insert new one.
  - Always upsert the `races` value.
- On purge: remove from `races` and corresponding `races_by_time` key.

Concurrency:
- Index updates use a read‑modify‑write guarded by per‑key ordering. If `started_at` changes concurrently, last‑write‑wins by `updated_at`.

## 7. Deletion & Secure Purge

- Delete (user‑facing): removes from in‑memory active set only; persisted history is retained.
- Secure Purge (admin‑only): `POST /admin/purge`
  - Request: `{ "race_ids": ["..."], "reason": "...", "requested_by": "..." }`
  - Effect: delete from `races` and related indexes; propagate to archival tiers and JSON snapshots; write audit record `meta/audit/<uuid>` with before/after counts.
  - Auth: required (implementation detail, out of scope here).

HTTP contract:
- Path: `/admin/purge`
- Method: `POST`
- Auth: Bearer token or mTLS (deployment choice). Unauthorized → `401`.
- Read‑only mode → `503 Service Unavailable` with headers `X-Raceboard-Read-Only: 1`, optional `Retry-After`.
- Accepted → `202` with `{ job_id }` if asynchronous; or `200` with `{ purged: [ids], not_found: [ids] }` if synchronous.
- Errors:
  - `400` invalid payload; `403` forbidden; `409` conflicting operation in progress; `500` internal.

Error payload examples:
```
// 401 Unauthorized
{ "error": "unauthorized", "message": "Bearer token missing or invalid" }

// 403 Forbidden
{ "error": "forbidden", "message": "Insufficient privileges" }

// 400 Bad Request
{ "error": "invalid_request", "message": "race_ids must be a non-empty array" }

// 409 Conflict
{ "error": "conflict", "message": "Another purge job is running", "job_id": "purge_abc123" }

// 503 Read-only
{ "error": "read_only", "message": "Server is in read-only mode" }

// 500 Internal
{ "error": "internal", "message": "Unexpected error; see server logs" }
```

## 8. Historic Query API (Pagination)

- Request params: `source?`, `from?`, `to?`, `limit?` (default 100, max 1000), `cursor?`, `include_events?`.
- Sort order: ascending by `started_at`, then `id` for tie‑break.
- Cursor: opaque Base64 of `{ last_started_at, last_id }`.
- Response: `{ items: [...], next_cursor?: "..." }`.

Example:

Request: `GET /historic/races?source=cargo&from=2025-09-01T00:00:00Z&limit=100`

Response:
```
{
  "items": [ { "id": "...", "source": "cargo", "title": "...", "started_at": "...", ... } ],
  "next_cursor": "eyJsYXN0X3N0YXJ0ZWRfYXQiOiIyMDI1LTA5LTAxVDAxOjAwOjAwWiIsICJsYXN0X2lkIjoiYWJiYyJ9"
}
```

Errors:
- `400` invalid cursor; `416` limit too large; `500` internal.

### 8.1 Internal Scan API (for Rebuild)

For clustering rebuilds on large datasets, avoid loading all races into memory. Use a streaming, filterable scan that reads in bounded batches ordered by time.

- Endpoint (internal API in PersistenceLayer):
  - `scan_races(filter: RaceScanFilter, batch_size: usize) -> RaceBatchStream`
- Filter:
  - `source?: String`, `from?: DateTime`, `to?: DateTime`, `include_events?: bool` (default false)
- Ordering: ascending by `started_at`, then `id`.
- Backing index: `races_by_time` to avoid full‑table scans.
- Stream contract:
  - Emits `Vec<Race>` batches (default `batch_size` 10_000) to keep memory bounded.
  - Best‑effort snapshot isolation is not guaranteed; entries may be eventually consistent.
  - Cursor resume: `cursor` is the last emitted `{started_at, id}` encoded Base64; passing it resumes from the next key.
- Deprecation: `get_all_races()` must not be used for rebuilds on large datasets; prefer `scan_races`.

## 9. Backups & Disaster Recovery

- JSON Snapshots: created daily (UTC) to `~/.raceboard/races.snapshot.json.zst` with SHA‑256 checksums; retention 30 days.
- Contents: unredacted to preserve rebuild integrity; restrict file permissions to user (0600). Provide a separate sanitized export for sharing if needed.
- Restore: documented in the DR playbook; includes integrity verification and controlled import path.

Snapshot maintenance:
- On snapshot creation: also persist `meta/snapshot/<timestamp>` with counts and SHA‑256; verify on next startup.

Restore runbook (summary):
- Stop server; verify snapshot checksum; import into a temporary DB; run verification scan; swap paths and restart; monitor metrics.


## 10. Security

- File Permissions: `~/.raceboard/*` default 0600.
- Encryption at Rest: optional (out of scope for this doc); if enabled, key management documented separately.
- Redaction: do not redact DR snapshots; provide separate sanitized exports when required.

Sanitized export (optional):
- Endpoint: `/admin/export/sanitized?ranges=...`
- Fields: redact `title` and `metadata` values, keep keys; preserve `id`, `source`, `state`, `started_at`, `duration`.

## 11. Compaction & Disk Hygiene

- Triggers: size growth > 20% in 24h, or tombstones > 10% of entries.
- Schedule: off‑peak by default; configurable; expose compaction metrics and last success timestamp.
- Impact: throttle IO to keep p95 read latency under 50 ms during compaction.

Operator controls:
- Manual trigger endpoint (admin): `/admin/compact` (POST) → `202` accepted; exposes progress via metrics and logs.

Compaction responses:
```
// 202 Accepted
{ "status": "accepted", "job_id": "compact_20250901_0100Z" }

// 409 Conflict (already running)
{ "error": "conflict", "message": "Compaction already in progress", "job_id": "compact_20250901_0100Z" }
```

## 12. Monitoring, Metrics & SLOs

- SLOs: write_success_rate ≥ 99.99%/5m, p95_write_latency ≤ 25 ms, p99_flush_latency ≤ 200 ms, compaction_duration ≤ 10 m, read_only_mode_active=0 outside maintenance.
- Metrics: `sled_db_size_bytes`, `tree_counts{tree}`, `write_latency_ms{p50,p95,p99}`, `flush_failures_total`, `serialize_failures_total`, `deserialize_failures_total`, `compaction_seconds_total`, `compaction_last_success_timestamp`, `json_snapshot_success_total`, `json_snapshot_last_success_timestamp`, `read_only_mode_active`, `purge_requests_total`, `purge_failures_total`.

Recommended alerts:
- Any `flush_failures_total` increase within 5 minutes.
- `deserialize_failures_total` > 0 within 5 minutes.
- `sled_db_size_bytes` growth > 25% day‑over‑day.
- `read_only_mode_active` toggled to 1 outside maintenance window.
- Snapshot missed at scheduled time.

Metric types (Prometheus):
- Gauges: `sled_db_size_bytes`, `tree_counts{tree}`, `read_only_mode_active`.
- Summaries/Histograms: `write_latency_ms`, `flush_latency_ms`, `compaction_seconds_total` (histogram).
- Counters: `flush_failures_total`, `serialize_failures_total`, `deserialize_failures_total`, `json_snapshot_success_total`, `purge_requests_total`, `purge_failures_total`.

## 13. Migration Plan (to v2 layout)

- Preconditions: stop server; backup sled dir and JSON snapshot; exclusive lock.
- Steps:
  1) Create trees `clusters`, `source_stats`, `meta` if missing.
  2) Clusters: copy valid `RaceCluster` records from root into `clusters` by `cluster_id`.
  3) Source stats: move `source:*` from root into `source_stats` using suffix as key.
  4) Races: union `races` tree with JSON snapshot; conflict resolution: prefer newer `updated_at` (or `started_at` if absent); then prefer record with more populated fields; cap events.
  5) Verify counts and sample decode; write `meta/schema_version=2`, `meta/migrated_at`, and a `migration_report` entry in `meta`.
  6) Dual‑read window: legacy reads allowed for one release; log legacy hits.
  7) Cleanup: purge legacy root/prefix keys after 0 legacy reads for 24h and one successful rebuild cycle.
- Rollback: restore backups and redeploy previous binary. Migration is additive and idempotent.

Migration report (stored at `meta/migrations/<timestamp>`):
```
{
  "schema_version": 2,
  "migrated_at": "2025-09-01T00:00:00Z",
  "counts": { "races": 123456, "clusters": 987, "source_stats": 5 },
  "skipped": { "clusters_corrupt": 2, "stats_corrupt": 0 },
  "json_merged": 3456,
  "sample_checksums": [ { "tree": "races", "key": "...", "crc32": 123456789 } ]
}
```

Acceptance criteria:
- Post‑migration, counts in new trees match (or exceed, when merging JSON) legacy totals; zero deserialize errors in logs; `schema_version=2` set; dual‑read yields zero legacy hits after 24h; rebuild completes successfully using `scan_races`.

## 15. CI Acceptance Checklist

- Storage layout
  - Creates trees `races`, `clusters`, `source_stats`, `meta` at first run; re-run is idempotent.
  - Envelope round-trip: can serialize/deserialize sample Race/Cluster/SourceStats with checksum verification.
- Write/flush SLOs
  - p95 write latency ≤ 25 ms and p99 flush latency ≤ 200 ms under smoke load (100 rps) in CI environment.
  - No `flush_failures_total` increments.
- Rebuild streaming
  - Populate ≥ 1,000,000 synthetic races across sources; `scan_races` processes in batches (e.g., 10k) with bounded memory (< 256 MB RSS increase).
  - Cursor resume yields identical sequence as uninterrupted scan.
- Pagination API
  - `/historic/races` enforces default and max limits; invalid cursor returns 400; large limit returns 416.
  - Cursor chaining returns complete, non-overlapping coverage within requested time range.
- Purge & read-only
  - `/admin/purge` returns 200 with purged/not_found sets; 401/403 for unauth; 503 in read-only with header `X-Raceboard-Read-Only: 1`.
  - Purge removes index entries (`races_by_time`) and tree values; audit record written in `meta/audit/*`.
- Migration
  - Migration tool produces `meta/migrations/<ts>` report with counts; post-migration read paths prefer new trees; legacy reads logged then stop within 24h.
  - Rollback plan validated: restore snapshot → previous binary passes read/write checks.

## 14. Compatibility Notes (Current Code vs Spec)

- Create Paths: HTTP writes JSON; gRPC writes memory only. Spec requires sled write‑through (both paths) with idempotency.
- Deletes: HTTP DELETE removes from sled; spec keeps history and restricts deletion to in‑memory active races. Add admin purge for compliance.
- Trees: clusters and source stats currently stored in root/prefix keys; migration moves them to dedicated trees.
- Startup: code loads clusters/stats; spec allows optional race warmup. Historic queries will move from JSON to sled with pagination.
- Locking: code falls back to PID/in‑memory DB on lock failure; spec exits or runs read‑only by flag.

Reason we store each field (minimalism):
- Race fields are required for UI rendering, filtering, and ETA context; events are capped and final duration is persisted for stats.
- Cluster fields are required for centroid computation and ETA predictions; member lists and histories are bounded to reduce memory and disk footprint while preserving signal.
- SourceStats provide fallback ETA and trend smoothing when clusters are insufficient.

Read‑only mode responses:
- Mutating endpoints return `503` with `X-Raceboard-Read-Only: 1` and optional `Retry-After` header.
- Health endpoint includes `read_only_mode_active` flag and a message.
