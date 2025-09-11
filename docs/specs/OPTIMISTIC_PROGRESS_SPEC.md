# Optimistic Progress Specification (Canonical)

Last updated: 2025-09-11

Replaces:
- OPTIMISTIC_PROGRESS_SUPPORT.md (server fields/how-to)
- OPTIMISTIC_PROGRESS_GRPC_AND_COMPATIBILITY.md (proto + adapter notes)
- OPTIMISTIC_PROGRESS_VALIDATION.md (validation checklist)

## Server Model
- Race fields: `last_progress_update`, `last_eta_update`, `eta_history` (recent revisions), `eta_source`, `eta_confidence`, `update_interval_hint`.
- All fields are optional and backward compatible; JSON-first persistence.
- Auto-infer `eta_source`/`eta_confidence` when missing (Exact/Adapter/Cluster/Bootstrap).

## gRPC Mapping
- Extend `Race` message to include the above fields; keep them optional.
- Keep `EtaRevision` with `eta_sec`, `timestamp`, `source`, `confidence`.

## UI Guidance
- Dual‑rail: solid = server; striped overlay = optimistic prediction; never regress solid.
- Trust window: activate overlay only when `now - last_progress_update > update_interval_hint` and source is not Exact.
- ETA presentation: show `≈` when overlay is active or ETA is predicted; show revision pill on increases.

## Validation Checklist
- Update timestamps set on changes; history capped to small N (e.g., 5).
- Trust window honored; overlay retracts smoothly on server updates.
- Calendar/exact ETAs never show prediction.

