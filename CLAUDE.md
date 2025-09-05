# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Raceboard Server** is a Rust-based local API server component of the Raceboard productivity tool. It tracks and manages "races" (jobs/tasks that take time like CI pipelines, builds, tests, deployments) through a REST API running on `http://localhost:7777`.

The server is part of a proof-of-concept system that includes:
- Swift/SwiftUI macOS client for visualization
- This Rust API server for race management
- Rust adapters for various integrations

## Build and Development Commands

```bash
# Build the project
cargo build

# Build for release
cargo build --release

# Run the server
cargo run

# Check compilation without building
cargo check

# Run tests
cargo test

# Run a specific test
cargo test test_name

# Format code
cargo fmt

# Check formatting without applying
cargo fmt --check

# Run linter
cargo clippy
```

## API Endpoints

The server implements the following REST API endpoints:

- `POST /race` - Create or update a race
- `PATCH /race/:id` - Patch specific race fields
- `POST /race/:id/event` - Append an event to a race
- `GET /races` - List all races
- `GET /race/:id` - Get a single race

## Race Data Structure

Each race contains:
- `id`: unique identifier
- `source`: origin system (e.g., "agent", "cmd")
- `title`: display name
- `state`: queued|running|passed|failed|canceled
- `started_at`: ISO 8601 timestamp
- `eta_sec`: estimated time to completion in seconds
- `progress`: 0-100 progress percentage
- `deeplink`: URL for direct access (file://, https://, vscode://)
- `metadata`: key-value pairs for additional data

## Current Implementation Status

The repository is in early proof-of-concept stage. The main.rs currently contains only a basic "Hello, world!" placeholder. The actual server implementation with HTTP endpoints, race management, and state storage needs to be developed.