# Roadmap (Prioritized)

Last updated: 2025-09-11

## P0 — Correctness and Ops
- Enforce read‑only on all gRPC mutators (done; verify via tests).
- Snapshot/restore helpers for sled; document backup procedures.
- Storage metrics and alerts (evictions, failed flushes, time‑index repairs).

## P1 — UX and Developer Experience
- Dual‑rail everywhere: update compact views or deprecate them.
- Unify ETA inference helpers used by HTTP/gRPC codepaths.
- Improve adapter examples and quickstart scripts.

## P2 — Tooling and Releases
- Release automation: tagging, changelog, GitHub releases.

## P3 — Extended Integrations
- Additional adapters: GitHub Actions, Jenkins, CircleCI.
- Cluster management UI (import/export/visualization).

References: `ARCHITECTURE_REVIEW.md`, `SERVER_IMPLEMENTATION.md`, `../specs/DATA_LAYER_SPECIFICATION.md`.
