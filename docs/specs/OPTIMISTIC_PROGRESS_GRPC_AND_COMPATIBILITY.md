# Optimistic Progress: gRPC Changes & Adapter Compatibility

## 1. gRPC Proto Changes

### Updated race.proto

```protobuf
syntax = "proto3";

package raceboard;

import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";

// Enum for ETA source/confidence
enum EtaSource {
  ETA_SOURCE_UNSPECIFIED = 0;
  ETA_SOURCE_EXACT = 1;        // Known end time (e.g., calendar events)
  ETA_SOURCE_ADAPTER = 2;       // Provided by adapter (estimated)
  ETA_SOURCE_CLUSTER = 3;       // Server prediction from clusters
  ETA_SOURCE_BOOTSTRAP = 4;     // Server bootstrap default
}

// Represents a single race with optimistic progress support
message Race {
  string id = 1;
  string source = 2;
  string title = 3;
  RaceState state = 4;
  google.protobuf.Timestamp started_at = 5;
  optional int64 eta_sec = 6;
  optional int32 progress = 7;
  optional string deeplink = 8;
  map<string, string> metadata = 9;
  repeated Event events = 10;
  
  // NEW: Optimistic Progress Support
  optional google.protobuf.Timestamp last_progress_update = 11;
  optional google.protobuf.Timestamp last_eta_update = 12;
  optional EtaSource eta_source = 13;  // How ETA was determined
  optional double eta_confidence = 14;  // 0.0-1.0 confidence score
  optional int32 update_interval_hint = 15;  // Expected seconds between updates
  repeated EtaRevision eta_history = 16;  // Recent ETA changes
}

// Track ETA changes
message EtaRevision {
  int64 eta_sec = 1;
  google.protobuf.Timestamp timestamp = 2;
  EtaSource source = 3;
  optional double confidence = 4;
}
```

### Key Additions Explained

1. **`eta_source`**: Indicates reliability of ETA
   - `EXACT`: Calendar events with known end times
   - `ADAPTER`: Adapter's estimate (e.g., GitLab pipeline)
   - `CLUSTER`: Server's ML prediction
   - `BOOTSTRAP`: Default fallback

2. **`eta_confidence`**: 0.0-1.0 score
   - 1.0 for exact calendar events
   - 0.7-0.9 for good cluster predictions
   - 0.3-0.6 for adapter estimates
   - 0.1-0.3 for bootstrap

3. **`update_interval_hint`**: Expected update frequency
   - Helps UI decide when to activate prediction

## 2. Adapter Compatibility Analysis

### Google Calendar Adapter - NO CHANGES NEEDED ✅

The calendar adapter provides **exact** ETAs and should mark them as such:

```rust
// In raceboard_calendar.rs
let race = Race {
    id: format!("gcal:free:{}-{}", start_str, end_str),
    source: "google-calendar".to_string(),
    title: "Free time".to_string(),
    state: RaceState::Running,
    started_at: window_start.to_rfc3339(),
    eta_sec: Some((window_end - now).num_seconds().max(0)),
    progress: Some(((elapsed as f64 / dur as f64) * 100.0) as i32),
    
    // NEW FIELDS - adapter can optionally set these
    eta_source: Some(EtaSource::Exact),  // This is EXACT time
    eta_confidence: Some(1.0),           // 100% confidence
    update_interval_hint: Some(30),      // Updates every 30s
    // last_progress_update: None,       // Server will set this
    // last_eta_update: None,            // Server will set this
    // eta_history: vec![],              // Server manages this
    ..Default::default()
};
```

### Backward Compatibility Strategy

**All new fields are OPTIONAL**, so existing adapters continue to work:

```rust
// OLD adapter code still works
let race = Race {
    id: "gitlab-123".to_string(),
    eta_sec: Some(120),  // Still works!
    progress: Some(45),  // Still works!
    // New fields default to None/empty
    ..Default::default()
};
```

### Server-Side Auto-Detection

The server can infer `eta_source` if not provided:

```rust
impl Race {
    pub fn infer_eta_source(&mut self) {
        if self.eta_source.is_none() && self.eta_sec.is_some() {
            self.eta_source = Some(match self.source.as_str() {
                "google-calendar" => EtaSource::Exact,
                "gitlab" | "github" | "jenkins" => EtaSource::Adapter,
                _ => EtaSource::Adapter,
            });
        }
    }
    
    pub fn infer_eta_confidence(&mut self) {
        if self.eta_confidence.is_none() && self.eta_source.is_some() {
            self.eta_confidence = Some(match self.eta_source.unwrap() {
                EtaSource::Exact => 1.0,
                EtaSource::Adapter => 0.5,
                EtaSource::Cluster => 0.7,
                EtaSource::Bootstrap => 0.2,
                _ => 0.3,
            });
        }
    }
}
```

## 3. Server Implementation Changes

### Update Tracking in gRPC Service

```rust
// src/grpc_service.rs
impl RaceService for RaceServiceImpl {
    async fn update_race(
        &self,
        request: Request<UpdateRaceRequest>,
    ) -> Result<Response<Race>, Status> {
        let update = request.into_inner();
        let mut race = self.storage.get_race(&update.id)
            .await
            .ok_or_else(|| Status::not_found("Race not found"))?;
        
        // Track update timestamps
        if let Some(new_progress) = update.progress {
            if race.progress != Some(new_progress) {
                race.last_progress_update = Some(Utc::now());
            }
        }
        
        if let Some(new_eta) = update.eta_sec {
            if race.eta_sec != Some(new_eta) {
                race.last_eta_update = Some(Utc::now());
                
                // Add to history
                race.eta_history.push(EtaRevision {
                    eta_sec: new_eta,
                    timestamp: Some(Utc::now().into()),
                    source: race.eta_source.unwrap_or(EtaSource::Adapter),
                    confidence: race.eta_confidence,
                });
                
                // Keep only last 5
                if race.eta_history.len() > 5 {
                    race.eta_history.remove(0);
                }
            }
        }
        
        // Apply updates
        race.apply_update(update);
        
        // Auto-infer if not set
        race.infer_eta_source();
        race.infer_eta_confidence();
        
        self.storage.store_race(&race).await;
        Ok(Response::new(race))
    }
}
```

## 4. UI Usage Examples

### Swift/SwiftUI Client

```swift
// Check ETA reliability
extension Race {
    var isExactEta: Bool {
        etaSource == .exact
    }
    
    var shouldShowPrediction: Bool {
        guard state == .running else { return false }
        guard let lastUpdate = lastProgressUpdate else { return true }
        
        let staleness = Date.now.timeIntervalSince(lastUpdate.dateValue())
        let trustWindow = TimeInterval(updateIntervalHint ?? 10)
        
        return staleness > trustWindow && !isExactEta
    }
    
    var etaConfidenceLevel: String {
        guard let confidence = etaConfidence else { return "Unknown" }
        switch confidence {
        case 0.9...1.0: return "Exact"
        case 0.7..<0.9: return "High"
        case 0.4..<0.7: return "Medium"
        default: return "Low"
        }
    }
}
```

### Handling Calendar Events Specially

```swift
// In progress bar view
if race.etaSource == .exact {
    // Exact ETA - no prediction needed, no "≈" symbol
    ProgressBar(
        progress: race.progress ?? 0,
        showPredictiveOverlay: false,  // No prediction overlay needed
        label: "ETA \(formatTime(race.etaSec))"  // "ETA 2h 15m" - NO "≈"
    )
} else {
    // Estimated ETA - show prediction when stale
    ProgressBar(
        progress: race.progress ?? 0,
        showPredictiveOverlay: race.shouldShowPrediction,
        overlayAmount: calculatePredictiveProgress(race),
        label: race.shouldShowPrediction ? "ETA ≈\(formatTime(race.etaSec))" : "ETA \(formatTime(race.etaSec))"
    )
}
```

## 5. Testing Strategy

### Test Matrix

| Adapter | eta_source | Behavior | UI Display |
|---------|------------|----------|------------|
| Google Calendar | EXACT | No prediction overlay | "ETA 2h 15m" (no "≈"), solid bar only |
| GitLab | ADAPTER | Prediction after stale | "ETA 45s" → "ETA ≈45s" + overlay after 10s |
| Custom CMD | None (inferred) | Prediction after stale | "ETA 3m" → "ETA ≈3m" + overlay after stale |
| New Race | BOOTSTRAP | Low confidence | "ETA ≈5m" always, overlay when running |

### Backward Compatibility Tests

```rust
#[test]
fn test_old_adapter_compatibility() {
    // Old adapter sends minimal data
    let update = UpdateRaceRequest {
        id: "test-123".to_string(),
        eta_sec: Some(120),
        progress: Some(50),
        // No new fields
        ..Default::default()
    };
    
    let race = service.update_race(update).await.unwrap();
    
    // Server should auto-populate
    assert!(race.eta_source.is_some());
    assert!(race.eta_confidence.is_some());
    assert!(race.last_progress_update.is_some());
}
```

## 6. Migration Plan

### Phase 1: Server Support (No Breaking Changes)
1. Add new optional fields to proto
2. Implement auto-inference for missing fields
3. Track update timestamps server-side
4. Deploy server update

### Phase 2: UI Adoption
1. Detect new fields availability
2. Use exact ETA flag for calendar events
3. Implement dual-rail based on confidence
4. Show "≈" only for non-exact ETAs

### Phase 3: Adapter Enhancement (Optional)
1. Update high-value adapters (GitLab, GitHub)
2. Add confidence scoring
3. Provide update interval hints

## 7. Summary

### Key Points
- ✅ **No breaking changes** - all new fields optional
- ✅ **Calendar adapter unchanged** - server detects exact ETAs
- ✅ **Progressive enhancement** - UI works better with new fields
- ✅ **Backward compatible** - old adapters continue working

### Benefits
- Calendar events show **exact** remaining time ("ETA 2h 15m") without "≈" symbol or prediction overlay
- CI/CD pipelines show **dual-rail** progress and "≈" when stale
- UI can distinguish between reliable and estimated ETAs
- Users always see **time remaining** (not end time), with confidence indicators

### Next Steps
1. Update proto file with optional fields
2. Implement server-side tracking and inference
3. Test with existing adapters
4. Update UI to use new fields when available