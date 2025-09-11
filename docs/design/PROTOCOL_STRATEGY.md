# Protocol Strategy: gRPC for UI, REST for Adapters (2025-09-10)

This document codifies the protocol boundary for Raceboard and aligns the project vision and documentation.

Decision summary:
- UI clients use gRPC only.
  - Purpose: low-latency streaming (StreamRaces), typed schemas, bidirectional compatibility.
  - Scope: read-only operations and status queries for UI/ops. No mutations via gRPC.
- Adapters use REST only.
  - Purpose: simpler integration surface, language-agnostic, easy retries, well-known semantics.
  - Scope: all write/mutation paths (create/update races, add events) and adapter lifecycle/health.

Implications:
- gRPC remains the canonical surface for UI streaming and system status reads.
- REST remains the canonical surface for ingestion from adapters (writes) and adapter health.
- Any gRPC adapter management RPCs are considered deprecated and will be removed after a migration window.

Current state vs target state:
- Current: The server exposes both REST and gRPC including adapter RPCs (RegisterAdapter, ReportAdapterHealth, DeregisterAdapter).
- Target: UI-only gRPC. Adapters exclusively via REST. Adapter gRPC RPCs removed or feature-gated off by default.

Why this split?
- Operational simplicity for adapters: REST is broadly accessible and easy to integrate from shells, CI systems, and diverse languages.
- Performance for UI: Streaming updates over gRPC provides low-latency, ordered updates ideal for dashboards.
- Clear responsibility boundaries reduce duplication and drift across protocol handlers.

Migration plan (high level):
1. Documentation update (this doc) and guidance across server and adapter docs. (You are here.)
2. Mark gRPC adapter RPCs as deprecated in docs and proto comments; discourage use in examples.
3. Enforce read-only on all remaining gRPC methods (no writes) and route adapter lifecycle/health to REST only.
4. Provide a compatibility period where gRPC adapter RPCs return UNIMPLEMENTED or a clear error when disabled via config flag.
5. Remove deprecated RPCs in the next major version.

Practical guidance:
- Building UIs:
  - Use RaceService.StreamRaces for real-time updates.
  - Use RaceService.GetRace/ListRaces and GetSystemStatus for views.
  - Do not call CreateRace/UpdateRace/AddEvent/DeleteRace from UI; these will become read-only or removed.
- Building adapters:
  - Use REST endpoints: POST /race, PATCH /race/{id}, POST /race/{id}/event, DELETE /race/{id} (rare), and adapter status endpoints.
  - Register/report health via REST adapter endpoints; avoid gRPC adapter RPCs.

FAQ:
- Can an adapter use gRPC for performance? Not recommended. The ingestion path is optimized for REST, and gRPC write methods will be deprecated.
- Will existing gRPC-based adapters break? We will provide a transition window with config flags and clear errors before removal.

Related docs:
- Server Guide: docs/SERVER_GUIDE.md
- Adapter Development Guide: docs/ADAPTER_DEVELOPMENT_GUIDE.md
- Architecture Review: docs/ARCHITECTURE_REVIEW.md
- gRPC schema: grpc/race.proto (see deprecation notes)
