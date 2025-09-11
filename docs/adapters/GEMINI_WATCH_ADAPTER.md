# Gemini Watch Adapter

Tails a Gemini CLI telemetry file and tracks prompts/responses as races.

- Binary: `raceboard-gemini-watch`
- Source: `src/bin/raceboard_gemini_watch.rs`
- Default telemetry file: `~/.gemini/telemetry.log`

## Usage

```
raceboard-gemini-watch [--server URL] [--telemetry-file PATH] [--eta SECS] [--from-start] [--no-events] [--poll-ms N] [--debug]
```

Key options:
- `-s, --server <URL>` — Raceboard server URL (default `http://localhost:7777`)
- `--telemetry-file <PATH>` — Telemetry file to tail (default `~/.gemini/telemetry.log`)
- `--eta <SECS>` — ETA hint in seconds for new sessions
- `--from-start` — Start reading from the beginning of the file (default tails from end)
- `--no-events` — Do not post per-event payloads; only lifecycle updates
- `--poll-ms <N>` — Poll interval in milliseconds (default 400)
- `--debug` — Verbose adapter logs

Examples:
```
# Tail default telemetry and post lifecycle + events
raceboard-gemini-watch

# Tail a specific file and run quietly
raceboard-gemini-watch --telemetry-file /tmp/gemini.telemetry --poll-ms 250
```

## Behavior
- Creates races with `source=gemini-cli`, infers titles from prompt text.
- Updates progress during tool use; completes races on success/failure events.
- Posts `Event` records unless `--no-events` is set.
- Registers adapter health as `adapter:gemini-watch:*` and reports periodically.

## See Also
- Server Guide: `../guides/SERVER_GUIDE.md`
- Data/ETA design: `../design/ETA_PREDICTION_SYSTEM.md`

