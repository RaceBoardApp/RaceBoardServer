# Raceboard Command Runner (Design Notes)

Last updated: 2025-09-11

This document captures the design intent for the `raceboard-cmd` adapter and defers day-to-day usage to the adapter docs.

See usage and options: `../adapters/CMD_RUNNER_ADAPTER.md`.

## Goals
- Track any local shell command as a race with minimal friction.
- Provide lightweight progress and rich context (stdout batches, working dir, command string).
- Fail-safe behavior: never block the command, tolerate server unavailability.

## Behavior (Summary)
- Creates a race (`source=cmd`), transitions `queued → running` when the process starts.
- Streams batched stdout lines as `Event{ event_type: "stdout" }` and completes with `passed/failed` on exit status.
- Optional ETA hint drives optimistic progress; otherwise progress remains event-driven.
- Metadata keys: `command`, `working_dir` (optional), user-provided pairs, optional `deeplink`.

## Reliability Notes
- Network failures: retries with jitter; drops non-critical events if the server is unreachable (race lifecycle is prioritized).
- Large outputs: batch and cap payload size to avoid oversized requests.

## Future Enhancements
- Auto-ETA hints for common commands (e.g., `cargo build`, `npm test`).
- Optional stderr capture as separate `Event{ event_type: "stderr" }`.
