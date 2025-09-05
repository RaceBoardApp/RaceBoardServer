# Repository Guidelines

## Project Structure & Module Organization
- `src/`: Core server code — `main.rs` (HTTP + gRPC startup), `handlers.rs` (REST), `grpc_service.rs` (Tonic service), `storage.rs`, `models.rs`, `config.rs`.
- `src/bin/`: Additional binaries — `raceboard-cmd`, `raceboard-claude`, `raceboard-track`, `raceboard-codex-watch`.
- `grpc/`: Protobufs (`race.proto`), compiled by `build.rs` via `tonic-build`.
- `api/openapi.yaml`: HTTP surface reference.
- `docs/`: Design notes and adapters.
- `config.toml`: Local defaults; environment can override.

## Build, Test, and Development Commands
- `cargo build`: Builds all binaries; runs proto codegen.
- `cargo run --bin raceboard-server`: Starts HTTP (`/health`, `/races`, `/race/{id}`) and gRPC services.
- `cargo test`: Runs async unit tests (Tokio).
- `cargo fmt` / `cargo clippy -- -D warnings`: Format and lint before PRs.

## Coding Style & Naming Conventions
- Use `rustfmt` defaults (4‑space indent). Keep functions/fields `snake_case`, types `CamelCase`, modules/files `snake_case`.
- Prefer small, testable modules; avoid long functions in handlers or service impls.
- Log with `log`/`env_logger`; avoid `println!` in runtime code.

## Testing Guidelines
- Place tests in `src/tests.rs` or module `#[cfg(test)]` blocks; use `#[tokio::test]` for async.
- Name tests `test_*` and keep them deterministic (e.g., Storage API CRUD, event limits).
- Run `cargo test -- --nocapture` when debugging failures.

## Commit & Pull Request Guidelines
- Commits: concise, present‑tense summaries (e.g., “Implement server persistence”). Group related changes.
- PRs: clear description, linked issues, steps to run (`cargo run --bin raceboard-server`), and sample checks (e.g., `GET /health`, grpcurl snippet). Include any config changes.

## Security & Configuration Tips
- Don’t commit secrets. Keep local overrides in `config.toml`.
- Override via env (double underscore separator):
  `RACEBOARD_SERVER__HTTP_PORT=8080 RACEBOARD_LOGGING__LEVEL=debug cargo run --bin raceboard-server`
- Rebuild after proto changes (`grpc/race.proto`): `cargo build` regenerates code through `build.rs`. Don’t edit generated modules.

