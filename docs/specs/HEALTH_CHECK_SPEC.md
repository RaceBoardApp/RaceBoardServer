# Adapter Health Check Specification (Canonical)

Last updated: 2025-09-11

This document specifies the adapter health model and REST API used by the server’s `AdapterRegistry`. It replaces the historical drafts:
- HEALTH_CHECK_UNIFIED_API.md (race-based approach; superseded)
- HEALTH_CHECK_SPECIFICATION_V2.md (early REST design)
- HEALTH_CHECK_DETAILED_SPECIFICATION.md (expanded draft)

## Goals
- Simple, localhost-only REST for adapters to register, report health, and deregister.
- Clear, time-bounded state machine with predictable transitions.
- Minimal persistence (registration + last metrics), no health-history firehose.

## States
- Initializing → Running → {Warning|Critical} → {Delayed → Absent → Abandoned}
- Stopped (graceful) and Exempt (non-reporting adapters like `claude`, `cmd`).
- Transitions are time-based using the adapter-provided interval with small grace.

## REST API
- POST `/adapter/register` → 201 Created
- POST `/adapter/health` → 200 OK
- DELETE `/adapter/register/{id}` → 204 No Content
- GET `/adapter/status` → list; GET `/adapter/status/{id}` → detail

Requests include adapter type, instance id, display name, capabilities, expected report interval, and minimal metrics (error/warn counts, CPU/mem optional). Responses include state and next expected report time.

Security: Bind server to localhost by default; optionally require a shared secret. Rate-limit registrations and health reports.

## Timing and Transitions (summary)
- Initializing: must send first report within 30s → TimedOut
- Running/Warning/Critical: if no report within 1.5×interval → Delayed
- Delayed: if no report within 2×interval (from last) → Absent
- Absent: if no report within 3×interval (from last) → Abandoned (requires re‑registration)
- Any non-terminal state + valid report → Running
- Stopped: terminal via explicit deregistration
- Exempt: no transitions; not expected to report

## Persistence
- Store registrations and last metrics in sled alongside server DB
- On startup: mark as Abandoned or use optimistic recovery with a short grace window; do not replay detailed health history

## Notes
- gRPC adapter RPCs are deprecated; adapters must use REST.
- UI reads aggregate status via REST or server-side summaries.

