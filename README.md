# 🏁 Raceboard

> A local-first productivity tool for tracking long-running tasks with ML-powered ETA predictions

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Swift](https://img.shields.io/badge/swift-F54A2A?style=flat&logo=swift&logoColor=white)](https://swift.org/)

## 🎯 Overview

Raceboard is a local-first productivity tool that tracks "races" - long-running tasks like CI pipelines, builds, deployments, tests, and other time-consuming processes. It provides real-time progress tracking with intelligent ETA predictions using machine learning.

### ✨ Key Features

- **🚀 Real-time Tracking** - Monitor progress of any long-running task via REST API and gRPC streaming
- **🧠 ML-Powered ETAs** - DBSCAN clustering with HNSW optimization learns from your historical data
- **📊 Optimistic Progress** - Dual-rail visualization prevents confusing progress jumps
- **🔌 Multiple Adapters** - GitLab CI, Google Calendar, Claude AI, and more
- **💾 Local-First** - All data stored locally with sled database
- **🎨 Beautiful UI** - Native macOS SwiftUI application with smooth animations

### Features in Action
- **ETA Revision Detection**: When ETAs increase, a "Revised ETA" pill appears
- **Trust Windows**: Smart prediction activation based on data freshness
- **Visual Clarity**: `ETA 2m 30s` (fresh) vs `ETA ≈2m 30s` (predicted)

## 🚀 Quick Start

### Installation
#### From Source
```bash
# Clone the repository
git clone https://github.com/RaceBoardApp/RaceBoardServer.git
cd RaceBoardServer

# Build and install
cargo build --release
./setup_raceboard.sh
```

### Basic Usage

1. **Start the server**:
```bash
raceboard-server
# Server runs on http://localhost:7777 (REST) and grpc://localhost:50051
```

2. **Track a build**:
```bash
# Start tracking a task
curl -X POST http://localhost:7777/race \
  -H "Content-Type: application/json" \
  -d '{
    "source": "build",
    "title": "Building my-app",
    "state": "running",
    "eta_sec": 180
  }'
```

3. **Use adapters**:
```bash
# GitLab CI adapter
raceboard-gitlab --config gitlab_config.toml

# Google Calendar adapter  
raceboard-calendar --config calendar_config.toml
```

## 🏗️ Architecture

```
┌─────────────────┐         ┌──────────────────┐         ┌─────────────┐
│   Adapters      │  REST   │  Raceboard      │  gRPC   │   UI Apps   │
│ • GitLab CI     │────────▶│     Server       │◀────────│  • macOS    │
│ • Calendar      │ /race   │                  │ Stream  │  • Terminal │
│ • Claude AI     │ PATCH   │  • Race Storage  │ :50051  │  • Web      │
│ • Codex         │ :7777   │  • ML Prediction │         │             │
└─────────────────┘         │  • Event System  │         └─────────────┘
                            │                  │
                            └──────────────────┘
                                     │
                                     ▼
                            ┌──────────────────┐
                            │   Persistence    │
                            │   • sled DB      │
                            │   • Clusters     │
                            │   • History      │
                            └──────────────────┘
```

### Core Components & Communication

#### Raceboard Server (Core)
- Listens on `localhost:7777` (REST) and `localhost:50051` (gRPC)
- Manages race lifecycle and state
- Performs ML-based ETA predictions
- Persists data to local sled database

#### Adapters → Server (REST API)
- Adapters push updates via `POST/PATCH http://localhost:7777/race`
- Create races, update progress, report completion
- Fire-and-forget pattern for reliability

#### UI Apps ← Server (gRPC Stream)
- UI subscribes to real-time updates via gRPC streaming
- Bidirectional: UI can also query and dismiss races
- Automatic reconnection on network issues

#### Data Flow
1. **External Service** (e.g., GitLab) → **Adapter** polls/webhooks
2. **Adapter** → **Server** via REST (create/update race)
3. **Server** → **Database** (persist state)
4. **Server** → **UI** via gRPC stream (real-time updates)
5. **UI** → **User** (visual progress with dual-rail)

## 📚 Documentation

### Getting Started
- [Server Guide](docs/guides/SERVER_GUIDE.md) - Complete server setup and operation
- [Configuration](docs/guides/CONFIGURATION.md) - Server and adapter configuration
- [API Reference](api/openapi.yaml) - OpenAPI specification

### Adapters
- [Adapter Development Guide](docs/adapters/ADAPTER_DEVELOPMENT_GUIDE.md) - Create custom adapters
- [GitLab Adapter](docs/adapters/GITLAB_ADAPTER.md) - GitLab CI integration
- [Google Calendar](docs/adapters/GOOGLE_CALENDAR_ADAPTER.md) - Calendar event tracking
- [Claude AI](docs/adapters/CLAUDE_ADAPTER.md) - AI assistant integration

### Advanced Features
- [ETA Prediction System](docs/design/ETA_PREDICTION_SYSTEM.md) - ML clustering details
- [Optimistic Progress](docs/specs/OPTIMISTIC_PROGRESS_SUPPORT.md) - Dual-rail visualization
- [Data Layer](docs/specs/DATA_LAYER_SPECIFICATION.md) - Persistence architecture

## 🛠️ Development

### Prerequisites
- Rust 1.70+ 
- macOS 13+ (for UI)
- Protocol Buffers compiler

### Building from Source

```bash
# Clone repository
git clone https://github.com/RaceBoardApp/RaceBoardServer.git
cd RaceBoardServer

# Build server
cargo build --release

# Run tests
cargo test

# Build with all features
cargo build --all-features --release
```

### Project Structure

```
raceboard/
├── src/                 # Server source code
│   ├── main.rs         # Server entry point
│   ├── grpc_service.rs # gRPC implementation
│   ├── handlers.rs     # REST API handlers
│   ├── prediction.rs   # ML prediction engine
│   └── bin/           # Adapter binaries
├── grpc/              # Protocol buffer definitions
├── docs/              # Documentation
├── tests/             # Integration tests
└── Raceboard UI/      # macOS SwiftUI app
```

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

### Areas for Contribution
- 🔌 New adapters (GitHub Actions, Jenkins, CircleCI)
- 🎨 UI improvements and themes
- 🧪 Test coverage improvements
- 📖 Documentation and examples
- 🐛 Bug fixes and performance improvements

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [Actix-web](https://actix.rs/) and [Tonic](https://github.com/hyperium/tonic)
- ML clustering powered by [HNSW](https://github.com/nmslib/hnswlib)
- Persistence via [sled](https://github.com/spacejam/sled)
- UI built with SwiftUI and Swift Concurrency

## 📊 Status

- ✅ Core server implementation
- ✅ Optimistic Progress v2
- ✅ ML-based ETA predictions
- ✅ GitLab, Calendar, Claude adapters
- ✅ macOS UI application
- 🚧 Installation packages
- 🚧 Additional adapters
- 📋 Cloud sync (planned)

## 📮 Contact & Support

- 🐛 [Report Issues](https://github.com/RaceBoardApp/RaceBoardServer/issues)
- 💬 [Discussions](https://github.com/RaceBoardApp/RaceBoardServer/discussions)
- 📖 [Documentation](https://github.com/RaceBoardApp/RaceBoardServer/wiki)

---

**Made with ❤️ for developers who wait for builds to complete**
