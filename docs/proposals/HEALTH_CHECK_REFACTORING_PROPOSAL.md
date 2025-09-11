# Health Check System Refactoring Proposal

## Executive Summary

This proposal outlines a refactoring of the Raceboard health check system from a **pull-based** model (server polls adapters) to a **push-based** model (adapters self-register and report their status). This architecture better aligns with the distributed nature of adapters and provides more flexibility for different adapter types.

**Key Insight:** Only persistent service adapters (GitLab, Calendar, Codex Watch, Gemini Watch) need registration and health checks. Ephemeral adapters (Claude, CMD) work through race creation/updates only.

## Current Problem

The initial implementation attempted to have the server poll each adapter for health status, which has several issues:

1. **Assumes adapters are always running** - But adapters may start/stop independently
2. **Requires known ports** - Server must know where each adapter is listening
3. **Claude adapter doesn't fit** - It works via hooks, not as a persistent service
4. **Network overhead** - Constant polling even when nothing changes
5. **Complex configuration** - Server needs to maintain adapter endpoint registry

## Proposed Solution: Self-Registration & Push Model

### Architecture Overview

```
┌─────────────┐     Register      ┌──────────────┐
│   Adapter   │ ──────────────────>│              │
│  (GitLab)   │                    │              │
│             │ Health Report      │    Server    │
│             │ ──────────────────>│              │
└─────────────┘    (periodic)      │              │
                                   │   Maintains  │
┌─────────────┐     Register      │   Adapter    │      Status Query    ┌──────────┐
│   Adapter   │ ──────────────────>│   Registry   │ <────────────────── │    UI    │
│  (Calendar) │                    │              │                      │          │
│             │ Health Report      │              │  Status Response    │          │
│             │ ──────────────────>│              │ ───────────────────> │          │
└─────────────┘                    └──────────────┘                      └──────────┘

┌─────────────┐
│   Claude    │  (No registration needed - works via hooks)
│   Adapter   │
└─────────────┘
```

## Adapter Categories

| Adapter | Type | Registration | Health Checks | Monitoring |
|---------|------|--------------|---------------|------------|
| **GitLab** | Persistent Service | ✅ Required | ✅ Every 30s | Status, metrics, uptime |
| **Calendar** | Persistent Service | ✅ Required | ✅ Every 60s | Status, metrics, uptime |
| **Codex Watch** | Persistent Service | ✅ Required | ✅ Every 30s | Status, metrics, uptime |
| **Gemini Watch** | Persistent Service | ✅ Required | ✅ Every 30s | Status, metrics, uptime |
| **Claude** | Hook-based | ❌ Not needed | ❌ Not needed | Activity via race creation |
| **CMD** | Ephemeral | ❌ Not needed | ❌ Not needed | Activity via race creation |

## Detailed Design

### 1. Adapter Registration API

New REST endpoint on the server:

```rust
POST /adapter/register
{
    "adapter_id": "gitlab",
    "display_name": "GitLab CI",
    "version": "1.0.0",
    "capabilities": {
        "supports_health_check": true,
        "health_check_interval_seconds": 30,
        "supports_metrics": true
    },
    "metadata": {
        "pid": 12345,
        "started_at": "2024-01-09T10:00:00Z",
        "config_path": "/path/to/config.toml"
    }
}

Response:
{
    "registration_id": "uuid-here",
    "server_time": "2024-01-09T10:00:00Z",
    "health_report_endpoint": "/adapter/health",
    "expected_report_interval": 30
}
```

### 2. Health Reporting API

Adapters push health updates:

```rust
POST /adapter/health
{
    "adapter_id": "gitlab",
    "registration_id": "uuid-here",
    "status": "healthy",
    "metrics": {
        "races_created": 42,
        "races_updated": 100,
        "last_activity": "2024-01-09T10:05:00Z",
        "error_count": 0
    },
    "next_report_in_seconds": 30
}
```

### 3. Adapter Lifecycle

#### Startup Sequence
1. Adapter starts and reads configuration
2. Adapter registers with server via `/adapter/register`
3. Server adds adapter to registry with status "running"
4. Adapter begins sending periodic health reports
5. Server updates adapter status based on reports

#### Shutdown Sequence
1. Adapter sends deregistration request (optional)
2. Or server marks adapter as "unknown" after missing health reports
3. UI shows adapter as offline

#### Crash Recovery
1. Server detects missing health reports (timeout = 2 × report interval)
2. Marks adapter status as "unknown" then "stopped" after longer timeout
3. When adapter restarts, it re-registers and status updates

### 4. Special Cases

#### Claude Adapter
- **No registration required** - Works through Claude Code hooks
- **No health checks** - Not a persistent service
- Server always shows Claude as "available" in status
- Activity tracked through race creation/updates only

#### Command Runner (raceboard-cmd)
- **No registration required** - Ephemeral, runs only during command execution
- **No health checks** - Exits immediately after command completes
- Activity tracked through race creation/updates only
- Server shows as "available" (since it can be invoked anytime)

### 5. Implementation Details

#### Server-Side Components

```rust
// adapter_registry.rs
pub struct AdapterRegistry {
    adapters: Arc<RwLock<HashMap<String, RegisteredAdapter>>>,
    last_update: Arc<RwLock<DateTime<Utc>>>,
}

pub struct RegisteredAdapter {
    pub adapter_id: String,
    pub registration_id: String,
    pub display_name: String,
    pub version: String,
    pub capabilities: AdapterCapabilities,
    pub status: AdapterStatus,
    pub last_health_report: DateTime<Utc>,
    pub metrics: AdapterMetrics,
    pub metadata: HashMap<String, String>,
}

pub struct AdapterCapabilities {
    pub supports_health_check: bool,
    pub health_check_interval_seconds: Option<u32>,
    pub supports_metrics: bool,
    pub supports_graceful_shutdown: bool,
}
```

#### Adapter-Side Library

Add to `adapter_common.rs`:

```rust
pub struct AdapterRegistration {
    server_url: String,
    adapter_info: AdapterInfo,
    registration_id: Option<String>,
    health_reporter: Option<HealthReporter>,
}

impl AdapterRegistration {
    pub async fn register(&mut self) -> Result<()> {
        // POST to /adapter/register
        // Store registration_id
        // Start health reporter if supported
    }
    
    pub async fn deregister(&self) -> Result<()> {
        // POST to /adapter/deregister
    }
    
    pub async fn report_health(&self, metrics: AdapterMetrics) -> Result<()> {
        // POST to /adapter/health
    }
}

// Automatic health reporting
pub struct HealthReporter {
    registration: Arc<AdapterRegistration>,
    interval: Duration,
}

impl HealthReporter {
    pub async fn start(self) {
        let mut ticker = interval(self.interval);
        loop {
            ticker.tick().await;
            let _ = self.registration.report_health(collect_metrics()).await;
        }
    }
}
```

### 6. Status API for UI

Enhanced gRPC method provides complete adapter status:

```protobuf
message AdapterStatus {
    string adapter_id = 1;
    string display_name = 2;
    AdapterState state = 3;  // REGISTERED, HEALTHY, UNHEALTHY, STOPPED, UNKNOWN
    string version = 4;
    google.protobuf.Timestamp registered_at = 5;
    google.protobuf.Timestamp last_health_report = 6;
    AdapterMetrics metrics = 7;
    map<string, string> metadata = 8;
    bool supports_health_check = 9;
    int32 seconds_since_last_report = 10;
}

message SystemStatus {
    repeated AdapterStatus adapters = 1;
    int32 total_registered = 2;
    int32 healthy_count = 3;
    ServerStatus server = 4;
}
```

## Benefits

### 1. **Flexibility**
- Adapters can run anywhere (not just known ports)
- Easy to add new adapter types
- Supports both persistent and ephemeral adapters

### 2. **Simplicity**
- No complex port management
- No polling infrastructure needed
- Clear separation of concerns

### 3. **Reliability**
- Adapters control their own health reporting
- Natural timeout detection for crashed adapters
- Graceful handling of network issues

### 4. **Scalability**
- Reduced network traffic (push vs poll)
- Server only tracks registered adapters
- Can handle many adapters efficiently

### 5. **Special Case Support**
- Claude adapter works without modification
- Command runner can be ephemeral
- Future adapters can opt-in to features

## Migration Plan

### Phase 1: Server Implementation (Week 1)
- [ ] Implement `/adapter/register` endpoint
- [ ] Implement `/adapter/health` endpoint  
- [ ] Create `AdapterRegistry` module
- [ ] Add cleanup job for stale registrations
- [ ] Update gRPC status method

### Phase 2: Adapter Common Library (Week 1)
- [ ] Add `AdapterRegistration` to `adapter_common.rs`
- [ ] Implement `HealthReporter` with automatic reporting
- [ ] Add graceful shutdown with deregistration
- [ ] Create usage examples

### Phase 3: Adapter Migration (Week 2)
- [ ] Update GitLab adapter to self-register
- [ ] Update Calendar adapter to self-register
- [ ] Update Codex/Gemini watchers to self-register
- [ ] Document Claude adapter exception (no registration)
- [ ] Document CMD runner exception (no registration)

### Phase 4: UI Integration (Week 2)
- [ ] Update UI to use new status API
- [ ] Show registration status for each adapter
- [ ] Handle "unknown" state gracefully
- [ ] Add manual refresh option

### Phase 5: Testing & Rollout (Week 3)
- [ ] Test registration/deregistration flow
- [ ] Test health timeout detection
- [ ] Test crash recovery
- [ ] Load test with many adapters
- [ ] Documentation update

## Example Usage

### Adapter Implementation

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Create adapter registration
    let mut registration = AdapterRegistration::new(
        &args.server,
        AdapterInfo {
            id: "gitlab".to_string(),
            display_name: "GitLab CI".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: AdapterCapabilities {
                supports_health_check: true,
                health_check_interval_seconds: Some(30),
                supports_metrics: true,
                supports_graceful_shutdown: true,
            },
        }
    );
    
    // Register with server
    registration.register().await?;
    
    // Start health reporting in background
    registration.start_health_reporter();
    
    // Main adapter logic
    run_gitlab_sync().await?;
    
    // Graceful shutdown
    registration.deregister().await?;
    Ok(())
}
```

### Server Status Query

```rust
// UI queries server for adapter status
let status = grpc_client.get_system_status().await?;

for adapter in status.adapters {
    println!("{}: {}", adapter.display_name, adapter.state);
    if adapter.supports_health_check {
        println!("  Last report: {} seconds ago", 
                 adapter.seconds_since_last_report);
    }
}
```

## Comparison Matrix

| Feature | Current (Pull) | Proposed (Push) |
|---------|---------------|-----------------|
| Adapter discovery | Pre-configured ports | Self-registration |
| Health monitoring | Server polls adapters | Adapters push status |
| Claude support | Doesn't fit model | Works naturally |
| Network traffic | Constant polling | Only on changes |
| Crash detection | Immediate (poll fails) | Timeout-based |
| Flexibility | Low | High |
| Complexity | High (server-side) | Low (distributed) |
| Scalability | Limited | Excellent |

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Adapters forget to register | Not visible in UI | Fallback detection via race creation |
| Network issues prevent health reports | False "unhealthy" status | Generous timeouts, retry logic |
| Registration ID conflicts | Duplicate adapters | UUID generation, validation |
| Memory leak from stale registrations | Server memory growth | Automatic cleanup job |

## Success Metrics

- [ ] All adapters successfully self-register
- [ ] Health status updates within 2 seconds
- [ ] Zero false "unhealthy" reports in 24 hours
- [ ] Claude adapter works without changes
- [ ] UI shows accurate adapter status
- [ ] Clean shutdown deregisters properly

## Conclusion

The push-based self-registration model provides a more flexible, scalable, and maintainable architecture for adapter health monitoring. It naturally handles different adapter types (persistent, ephemeral, hook-based) and reduces complexity while improving reliability.

This refactoring aligns with distributed system best practices and sets a solid foundation for future adapter ecosystem growth.