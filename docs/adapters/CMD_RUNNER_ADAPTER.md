# CMD Runner Adapter

Runs a local shell command and tracks it as a race with live stdout events.

- Binary: `raceboard-cmd`
- Source: `src/bin/cmd_runner.rs`
- Protocol: REST (writes) → Raceboard server at `http://localhost:7777` by default

## Usage

```
raceboard-cmd [OPTIONS] -- <command> [args...]
```

Common options:
- `-s, --server <URL>` — Raceboard server URL (default `http://localhost:7777`)
- `-t, --title <TEXT>` — Friendly title (defaults to the command string)
- `-e, --eta <SECS>` — ETA hint in seconds
- `-d, --working-dir <PATH>` — Working directory
- `-o, --output` — Also print command output to the console
- `-m, --metadata key=value` — Extra metadata (repeatable)

Examples:
```
# Track a build
raceboard-cmd -t "Build app" -e 180 -- cargo build --release

# Track tests and include working directory and metadata
raceboard-cmd -d ./server -m suite=unit -m ci=false -- cargo test -- --nocapture

# Provide a deep link
raceboard-cmd -l "vscode://file/${PWD}/src/main.rs" -- cargo clippy -- -D warnings
```

## Behavior
- Creates a race with `source=cmd`, starts in `queued` then `running`.
- Streams batched `stdout` lines as `Event { event_type: "stdout" }`.
- Updates progress opportunistically; completes with `passed`/`failed` based on exit code.
- Attaches `command`, `working_dir` (if provided), and user metadata to `race.metadata`.

## See Also
- Server Guide: `../guides/SERVER_GUIDE.md`
- HTTP API: `../guides/CONFIGURATION.md`, `../../api/openapi.yaml`

