# Day 2: Installation and Cluster Sharing

The goal for Day 2 is to deliver a clear plan for true one‑click installation on macOS and to continue laying groundwork for cluster sharing.

## One‑Click Installation Proposal (macOS App Store)

### Goal & Scope
- One click: the user installs RaceboardUI from the Mac App Store, launches it, selects adapters, and the UI provisions and runs the local server plus chosen adapters without Terminal.
- Platform: macOS, App Store distribution. No privileged helpers; everything runs in the user session.
- Constraints: App Sandbox; no downloading or executing new code post‑install; respect privacy prompts; persist data inside an app group container.

### User Flow (First Launch)
- Welcome → Install Server → Choose Adapters → Permissions → Finish.
- After Finish, the server listens on localhost; adapters start and appear in the UI status panel.

### Architecture
- UI App (RaceboardUI): sandboxed GUI from the App Store.
- Login Item Helper: background app launched via `SMAppService.loginItem`; manages server and adapters.
- Embedded Binaries: `raceboard-server` and adapters live under the helper bundle `Resources/bin/`, codesigned and notarized.
- Data Location: App Group container `~/Library/Group Containers/<teamid>.raceboard/` for sled DB, configs, logs, and tokens.
- IPC: HTTP `127.0.0.1:<http_port>` and gRPC `127.0.0.1:<grpc_port>`.

### Packaging
- Embed prebuilt Rust binaries (server and adapters) inside the helper bundle; do not fetch code at runtime (App Store policy).
- Codesign app, helper, and all embedded binaries; App Store notarizes on submission.
- App Store updates the UI, helper, and embedded binaries together; no separate updater required.

### Entitlements
- UI App: App Sandbox; `com.apple.security.app-sandbox`, `com.apple.security.network.client`, App Groups, Keychain Access Groups.
- Helper: App Sandbox; `network.client` + `network.server`; App Groups; Keychain Access Groups; `files.user-selected.read-only` to retain bookmarks for log directories.

### Server Provisioning
- Config file: write `config.toml` into the app group container; bind to `127.0.0.1` by default.
- Ports: probe `7777` / `50051`; if busy, pick the next free and persist; UI shows actual ports.
- Env overrides: launch with `RACEBOARD_SERVER__HTTP_PORT`, `RACEBOARD_SERVER__GRPC_PORT`, `RACEBOARD_LOGGING__LEVEL`, and `RACEBOARD_PERSISTENCE__PATH` pointing to the group container.
- Data path: sled DB at `<group>/eta_history.db` (via `persistence.path`).

### Adapters Provisioning
- Codex Log Watcher: onboarding asks the user to pick log folders; store security‑scoped bookmarks; launch watcher with `--watch` paths and `--server http://127.0.0.1:<port>`.
- Google/ICS Calendar: OAuth in the UI; tokens in Keychain + group files; launch adapter with `--config <group>/calendar_config.toml`.
- GitLab CI: host and token gathered in the UI; token in Keychain; launch adapter headless.
- Supervision: helper runs each adapter as a child process, restarts with backoff, logs to `<group>/logs/`.

### Install/Uninstall & Updates
- Start/Stop: UI toggles the login item via `SMAppService`; helper starts/stops server and adapters gracefully.
- Updates: App Store updates everything; helper performs DB/migration checks on first run after update.
- Uninstall: UI action to disable the login item and optionally delete the app group data (config/DB/logs) with confirmation.

### Security & Privacy
- Local‑only: server binds to `127.0.0.1` only.
- Least privilege: adapters only read folders the user granted; bookmarks can be revoked in UI.
- Secrets: tokens in Keychain; configs without secrets in the group container. Telemetry is opt‑in.

### Resilience
- Health checks: helper polls `/health`; restart unhealthy processes with capped backoff.
- Port conflict handling: auto‑fallback to free ports; UI displays chosen ports.
- Crash logs: rotate per process under `<group>/logs/`; optional user‑approved crash reports.

### Implementation Plan (Phased)
- Helper & Supervision
  - Create a macOS Login Item target; integrate `SMAppService.loginItem`.
  - Embed server/adapters in `Resources/bin/`; implement a supervisor (spawn, monitor, restart, structured logs).
- Server Config Support
  - Extend `Settings` to support `persistence.path` (env `RACEBOARD_PERSISTENCE__PATH`), and bind to loopback by default.
  - Helper probes ports and writes/exports them via env or config.
- UI Onboarding
  - Wizard pages: Welcome → Choose Adapters → Permissions → Review → Install.
  - Directory picker for log watcher with security‑scoped bookmarks; OAuth for Google.
- Adapter Standardization
  - Ensure all adapters accept `--server`/`--config` flags and read configs from the group path.
  - Optional: an adapter manager that reads a manifest and launches selected adapters to reduce process count.
- Status & Controls
  - UI status for server health, ports, and adapter states; restart buttons; open logs/data folder.
- QA & Release
  - Sandboxed runtime tests, migration tests, onboarding UX; App Store compliance checklist (no dynamic code, correct entitlements).

### Minimal Repo Changes Needed
- Settings: add `persistence.path` with env override `RACEBOARD_PERSISTENCE__PATH`; thread into `PersistenceLayer::new(Some(path))`.
- Default bind addresses in `config.toml`: `http_host = "127.0.0.1"`, `grpc_host = "127.0.0.1"` (already set).
- Verify adapters accept the standardized flags and can operate with files under the app group path.

---

## Cluster Sharing — Status
- ✅ Persistence (sled) supports cluster storage; export/import infra exists.
- ✅ Phased rollout system present.
- ❌ UI for managing shared clusters not yet implemented.

Next steps: add UI surfaces to export clusters to a portable bundle, import with review, and opt‑in sharing policies (source scoping, anonymization).
