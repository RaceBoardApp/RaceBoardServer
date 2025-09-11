# Server Configuration

The server loads defaults from `config.toml` in the repo root and allows overrides via environment variables.

## File: `config.toml`
Example keys (actual file may contain more):
```
[server]
http_port = 7777
grpc_port = 50051

[logging]
level = "info"
```

## Server Settings

Available keys in [server] table:
- http_host (string) — default: 127.0.0.1
- http_port (u16) — default: 7777
- grpc_host (string) — default: 127.0.0.1
- grpc_port (u16) — default: 50051
- read_only (bool) — default: false; if true, all mutating endpoints (HTTP and gRPC) are disabled.
- legacy_json_fallback_enabled (bool) — default: true; gates legacy ~/.raceboard/races.json fallback reads and writes. When false, handlers skip legacy JSON and only use sled (a backup is still written to ~/.raceboard/races.json.bak on completion events).

## Environment Overrides
Use the `RACEBOARD_` prefix and a double underscore (`__`) between table and key names.

Examples:
```
# Change HTTP port and log level
RACEBOARD_SERVER__HTTP_PORT=8080 \
RACEBOARD_LOGGING__LEVEL=debug \
cargo run --bin raceboard-server
```

## Adapter Configuration (Calendar)
The calendar adapter can be configured via a separate `calendar_config.toml` placed anywhere; pass `--config /path/to/file` to the binary.

ICS (no Google Cloud):
```
[raceboard]
server_url = "http://localhost:7777"

[ics]
url = "https://calendar.google.com/calendar/ical/<secret>/basic.ics"

[working_hours]
start = "10:00"
end   = "18:00"

[sync]
interval_seconds = 30

[filters]
ignore_all_day_events = true
```

Google OAuth:
```
[raceboard]
server_url = "http://localhost:7777"

[google]
credentials_path = "creds/client_secret.json"
token_cache      = "calendar_tokens.json"

[working_hours]
start = "10:00"
end   = "18:00"
```
