# Server Support for Optimistic Progress

## Quick Implementation Guide

This document outlines the server changes needed to support the Optimistic Progress v2 feature in the Raceboard UI.

## Phase 1: Essential Changes (Implement Now)

### 1. Update Race Model

Add to `src/models.rs`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Race {
    // ... existing fields ...
    
    // New fields for optimistic progress
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_progress_update: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_eta_update: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_eta_sec: Option<i64>, // For detecting increases
}
```

### 2. Update PATCH Handler

Modify `update_race` in `src/handlers.rs`:
```rust
pub async fn update_race(
    data: web::Data<AppState>,
    path: web::Path<String>,
    update: web::Json<RaceUpdate>,
) -> Result<HttpResponse> {
    let race_id = path.into_inner();
    
    // Get existing race
    if let Some(mut race) = data.storage.get_race(&race_id).await {
        // Track ETA changes
        if let Some(new_eta) = update.eta_sec {
            if race.eta_sec != Some(new_eta) {
                race.previous_eta_sec = race.eta_sec;
                race.last_eta_update = Some(Utc::now());
            }
        }
        
        // Track progress changes
        if let Some(new_progress) = update.progress {
            if race.progress != Some(new_progress) {
                race.last_progress_update = Some(Utc::now());
            }
        }
        
        // Apply update
        race.apply_update(update.into_inner());
        
        // Store and return
        data.storage.store_race(race.clone()).await;
        Ok(HttpResponse::Ok().json(&race))
    } else {
        Ok(HttpResponse::NotFound().finish())
    }
}
```

### 3. Add ETA History Tracking

Create a simple in-memory ETA history:
```rust
// In src/models.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtaRevision {
    pub eta_sec: i64,
    pub timestamp: DateTime<Utc>,
    pub confidence: Option<f64>,
}

// Add to Race struct
pub eta_revisions: Vec<EtaRevision>, // Keep last 5

// In update logic
if let Some(new_eta) = update.eta_sec {
    if race.eta_sec != Some(new_eta) {
        // Add to history
        race.eta_revisions.push(EtaRevision {
            eta_sec: new_eta,
            timestamp: Utc::now(),
            confidence: None, // Will be set if from prediction
        });
        
        // Keep only last 5
        if race.eta_revisions.len() > 5 {
            race.eta_revisions.remove(0);
        }
    }
}
```

## Phase 2: Enhanced Features

### 1. Update Interval Hints

Based on source clustering:
```rust
pub fn get_update_interval_hint(source: &str) -> i32 {
    match source {
        "gitlab" | "github" => 10,  // CI/CD updates every 10s
        "cmd" => 5,                 // Command line updates frequently
        "claude-code" => 15,        // AI tasks update less frequently
        _ => 10,                    // Default
    }
}
```

### 2. Progress Confidence

Add to prediction engine:
```rust
pub struct ProgressPrediction {
    pub current: i32,
    pub confidence: f64,
    pub predicted_next: Option<i32>,
    pub source: String, // "reported", "interpolated", "estimated"
}
```

## API Response Example

After Phase 1 implementation:
```json
{
  "id": "gitlab-pipeline-123",
  "source": "gitlab",
  "title": "Build Pipeline #123",
  "state": "running",
  "started_at": "2024-01-01T12:00:00Z",
  "progress": 45,
  "eta_sec": 120,
  "last_progress_update": "2024-01-01T12:02:30Z",
  "last_eta_update": "2024-01-01T12:02:00Z",
  "previous_eta_sec": 150,
  "eta_revisions": [
    {"eta_sec": 180, "timestamp": "2024-01-01T12:00:00Z", "confidence": 0.3},
    {"eta_sec": 150, "timestamp": "2024-01-01T12:01:00Z", "confidence": 0.5},
    {"eta_sec": 120, "timestamp": "2024-01-01T12:02:00Z", "confidence": 0.7}
  ]
}
```

## Migration Strategy

1. **Backward Compatibility**: All new fields are optional
2. **Gradual Rollout**: Clients can detect presence of new fields
3. **Fallback Behavior**: Clients work without new fields (degraded mode)

## Testing Checklist

- [ ] Update timestamps are set on PATCH
- [ ] ETA history maintains max 5 entries
- [ ] Previous ETA is tracked for increase detection
- [ ] All fields serialize/deserialize correctly
- [ ] Storage layer handles new fields
- [ ] API responses include new fields when present
- [ ] Backward compatibility with existing clients

## Benefits for UI

With these changes, the UI can:
1. **Determine Freshness**: Check `now - last_progress_update` for trust window
2. **Detect ETA Increases**: Compare current vs `previous_eta_sec`
3. **Show Revision History**: Display `eta_revisions` in tooltip
4. **Activate Prediction**: Use update timestamps to trigger optimistic mode
5. **Calculate Confidence**: Use revision history to assess prediction reliability

## Next Steps

1. Implement Phase 1 changes
2. Test with UI locally
3. Deploy to staging
4. Gather feedback
5. Consider Phase 2 enhancements