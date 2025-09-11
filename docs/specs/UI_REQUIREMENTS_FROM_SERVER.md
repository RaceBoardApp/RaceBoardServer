# UI Requirements from Server & Adapters

This document specifies exactly what the UI needs from the server and adapters to implement the complete Raceboard experience.

## 1. Server Requirements for UI

### 1.1 Data Fields Required (via gRPC)

The UI needs these fields in the Race message for optimistic progress:

```protobuf
message Race {
  // Core fields (existing)
  string id = 1;
  string source = 2;
  string title = 3;
  RaceState state = 4;
  google.protobuf.Timestamp started_at = 5;
  optional int64 eta_sec = 6;
  optional int32 progress = 7;
  optional string deeplink = 8;
  
  // REQUIRED for Optimistic Progress v2
  optional google.protobuf.Timestamp last_progress_update = 11;  // ✅ Implemented
  optional google.protobuf.Timestamp last_eta_update = 12;       // ✅ Implemented
  optional EtaSource eta_source = 13;                           // ✅ Implemented
  optional double eta_confidence = 14;                          // ✅ Implemented
  optional int32 update_interval_hint = 15;                     // ✅ Implemented
  repeated EtaRevision eta_history = 16;                        // ✅ Implemented
}
```

### 1.2 Server Behavior Requirements

#### Timestamp Tracking
- **last_progress_update**: Must update whenever `progress` field changes
- **last_eta_update**: Must update whenever `eta_sec` field changes
- These timestamps are critical for UI to determine data freshness

#### ETA Source Inference
Server MUST set `eta_source` based on adapter source:
- `"google-calendar"` → `ETA_SOURCE_EXACT` (value: 1)
- `"gitlab"`, `"github"`, `"jenkins"` → `ETA_SOURCE_ADAPTER` (value: 2)
- ML predictions → `ETA_SOURCE_CLUSTER` (value: 3)
- Default/bootstrap → `ETA_SOURCE_BOOTSTRAP` (value: 4)

#### Update Interval Hints
Server MUST provide `update_interval_hint` (seconds) for trust windows:
- EXACT sources: 60 seconds
- ADAPTER sources: 10 seconds
- CLUSTER sources: 15 seconds
- BOOTSTRAP sources: 10 seconds

#### ETA History
- Maintain last 5 ETA changes in `eta_history`
- Each revision must include: `eta_sec`, `timestamp`, `source`, `confidence`
- UI uses this to detect and announce ETA revisions

### 1.3 API Endpoints

#### gRPC Streaming (Primary)
```
service RaceService {
  rpc StreamRaces(Empty) returns (stream RaceUpdate);  // ✅ Implemented
}
```
- Must send initial snapshot of all races
- Must send incremental updates (CREATED, UPDATED, DELETED)
- UI relies on this for real-time updates

#### REST Endpoints (Secondary)
```
GET  /races           # List all races
POST /race           # Create race (adapters use this)
PATCH /race/:id      # Update race (adapters use this)
POST /race/:id/event # Add event to race
DELETE /race/:id     # Delete race (UI dismiss)
```

### 1.4 Server Configuration for UI

For App Store deployment, server MUST support:

```toml
# Environment variable overrides needed
RACEBOARD_SERVER__HTTP_PORT=7777          # UI needs to know actual port
RACEBOARD_SERVER__GRPC_PORT=50051         # UI connects here
RACEBOARD_PERSISTENCE__PATH=/custom/path  # For sandboxed data
RACEBOARD_LOGGING__LEVEL=info             # For debugging
```

## 2. Adapter Requirements for UI

### 2.1 Standardized Command-Line Interface

ALL adapters MUST accept these flags for UI integration:

```bash
# Required flags
--server <URL>        # Server URL (e.g., http://localhost:7777)
--config <PATH>       # Config file path

# Optional but recommended
--log-level <LEVEL>   # Logging verbosity
--health-port <PORT>  # Health check endpoint
--version            # Version information
--validate-config    # Validate config without running
```

### 2.2 Adapter-Specific Requirements

#### GitLab Adapter
- Must NOT delete races when pipelines complete (only update state)
- Must set proper final states: PASSED, FAILED, CANCELED
- Must provide `deeplink` to GitLab pipeline page
- Should provide meaningful `metadata` (branch, commit, etc.)

#### Google Calendar Adapter
- Must set `eta_source` to EXACT (or server infers from source name)
- Must provide exact `eta_sec` based on event end time
- Should never trigger prediction overlay (calendar times are exact)

#### Claude/Codex Adapters
- Must provide progress updates at reasonable intervals
- Should set `update_interval_hint` based on expected update frequency
- Must properly detect completion and set final state

### 2.3 Data Quality Requirements

Adapters MUST ensure:
- **Unique IDs**: Each race has globally unique ID
- **State Transitions**: Follow valid state machine (QUEUED → RUNNING → PASSED/FAILED/CANCELED)
- **Progress Monotonicity**: Progress should generally increase (server handles regression)
- **ETA Reasonableness**: ETAs should be positive and reasonable

## 3. Critical Integration Points

### 3.1 For Optimistic Progress to Work

UI needs ALL of these to function correctly:

1. **Timestamps**: `last_progress_update` and `last_eta_update` for freshness
2. **Source Classification**: `eta_source` to determine prediction behavior
3. **Update Hints**: `update_interval_hint` for trust window calculation
4. **ETA History**: For revision detection and announcement

### 3.2 For App Store Deployment

Server needs:
- Environment variable support for paths and ports
- Ability to bind to localhost only (security)
- Configurable persistence path for sandboxed data

Adapters need:
- Standardized CLI flags
- Config file support
- Operation within sandboxed environment

## 4. Current Implementation Status

### ✅ Complete
- All proto fields implemented in server
- Timestamp tracking on updates
- ETA source inference logic
- Update interval hint defaults
- ETA history with 5-item limit
- gRPC streaming with snapshot + incremental
- GitLab adapter fixed (no deletion)

### ⚠️ Needs Verification
- All adapters accept standardized flags
- Adapters work with custom config paths
- Server handles RACEBOARD_PERSISTENCE__PATH environment variable

### ❌ Not Implemented
- Server doesn't yet support persistence.path in config
- Some adapters may not have --config flag support

## 5. Testing Checklist for UI Integration

Before UI can work properly, verify:

- [ ] Server provides all required proto fields
- [ ] Timestamps update when progress/ETA changes
- [ ] ETA source is correctly inferred
- [ ] Update interval hints are reasonable
- [ ] ETA history accumulates (max 5)
- [ ] gRPC streaming works reliably
- [ ] Adapters accept standardized flags
- [ ] Calendar adapter gets EXACT source
- [ ] GitLab adapter doesn't delete races
- [ ] Server can use custom persistence path

## 6. Example Data Flow

1. **Adapter** detects CI pipeline started
2. **Adapter** POST to `/race` with initial data
3. **Server** adds timestamps, infers source, sets update hint
4. **Server** streams update to UI via gRPC
5. **UI** receives race with all optimistic progress fields
6. **UI** uses fields to determine:
   - Should prediction be shown? (check source, staleness)
   - Is data fresh? (check timestamps vs trust window)
   - Did ETA increase? (check history or compare)
   - Show approximation symbol? (check staleness)

This complete data flow enables the dual-rail progress visualization and ETA revision detection.