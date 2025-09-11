# Codex Watch Adapter

Tails the local Codex log and creates/updates races for interactive coding sessions.

- Binary: `raceboard-codex-watch`
- Source: `src/bin/raceboard_codex_watch.rs`
- Default log: `~/.codex/log/codex-tui.log`

## Usage

```
raceboard-codex-watch [--server URL] [--log-path PATH] [--poll-ms N] [--no-watcher] [--only-submission-starts] [--min-turn-secs N] [--debug]
```

Key options:
- `-s, --server <URL>` — Raceboard server URL (default `http://localhost:7777`)
- `--log-path <PATH>` — Override Codex log path (default `~/.codex/log/codex-tui.log`)
- `--poll-ms <N>` — Poll interval in milliseconds (default 500)
- `--no-watcher` — Disable filesystem watcher; use polling only
- `--only-submission-starts` — Only auto-start races on prompt submission
- `--min-turn-secs <N>` — Minimum seconds a turn must run before completion is honored (default 2)
- `-d, --debug` — Verbose adapter logs

Examples:
```
# Start with defaults (tail from end)
raceboard-codex-watch

# Custom log path and higher poll rate
raceboard-codex-watch --log-path ~/.codex/log/codex-tui.log --poll-ms 200

# Troubleshooting: enable Codex debug logs so events appear in the log
RUST_LOG=codex_core=debug,codex_tui=debug codex
```

## Behavior
- Detects prompt submissions and function calls to estimate progress and ETA.
- Creates races with `source=codex-session` and updates progress based on activity.
- Posts structured `Event`s for notable actions when available.
- Registers adapter health as `adapter:codex-watch:*` and reports periodically.

## See Also
- Adapter design: `../design/CODEX_LOG_TRACKING.md`
- Server Guide: `../guides/SERVER_GUIDE.md`

