# Data Integrity and Recovery Plan (sled)

## Critical Failure Analysis

### What Went Wrong
1. **Storage Layer Disconnect**: The in-memory storage (`Storage`) was never connected to the persistence layer
2. **Silent Data Loss**: When the 1000 race limit was hit, oldest races were silently deleted without persistence
3. **No Historic Data for ML**: Cluster rebuilding requires thousands of historic races, but we only kept 1000 in memory
4. **No Monitoring**: No alerts or logs when data was being discarded

### Impact on Cluster Rebuilding
- **DBSCAN needs minimum 100-1000 races per source** for meaningful clustering
- **Kneedle algorithm needs historic data** to detect optimal eps values
- **Silhouette scoring needs completed races** to validate cluster quality
- **Phased rollout needs historic performance data** to make decisions

## Recovery Strategy

### Phase 1: Immediate Data Recovery
1. Check if any data exists in `~/.raceboard/eta_history.db`
2. Check for any backup files or temporary databases
3. Recover from any available logs or external sources

### Phase 2: Architectural Fixes (aligned with current design)
1. Keep a small in-memory active set; persist only on completion (current two‑plane model).
2. Enforce a clear event cap per race and explicit `duration_sec` on completion.
3. Maintain a time index (`races_by_time`) for efficient historic scans.
4. Gate legacy JSON fallbacks behind config; prefer JSON‑first serialization in sled with bincode fallback.

### Phase 3: Monitoring & Alerts
1. **Data integrity checks** on startup
2. **Alert on data loss events**
3. **Periodic backup verification**
4. **Metrics for storage health**

## Implementation Plan

### 1. Storage Architecture Redesign
```rust
pub struct Storage {
    // Active races (recent, frequently accessed)
    active_races: RwLock<HashMap<String, Race>>,
    
    // Historic races (completed, for ML training)
    // Backed by persistence layer, not limited
    persistence: Arc<PersistenceLayer>,
    
    // Write-ahead log for crash recovery
    wal: Arc<WriteAheadLog>,
    
    // Metrics and monitoring
    metrics: StorageMetrics,
}
```

### 2. Data Flow
```
Create/Update Race -> WAL -> Persistence -> Memory Cache -> Response
                        |
                        v
                   Backup Queue
```

### 3. Backup Strategy
- **Primary**: sled database at `~/.raceboard/eta_history.db` (JSON-serialized values).
- **Snapshots**: Periodic zstd-compressed snapshots of `races`, `source_stats`, and `clusters` trees.
- **Exports**: JSON export/import tooling for clusters and source stats.
- **Location**: `~/.raceboard/backups/` with date-stamped files and retention policy.

### 4. Monitoring
- Storage capacity and sled lock acquisition failures.
- Data loss detection (unexpected time-index gaps) with repair on startup.
- Backup job success metrics and periodic restore validation in staging.
- Cluster rebuild sufficiency checks (minimum samples, noise ratio).

## Prevention Measures

### 1. Development Practices
- **Never silently discard data**
- **Always log data eviction**
- **Test with realistic data volumes**
- **Monitor storage in production**

### 2. Testing Requirements
- Test with 10,000+ races
- Verify cluster rebuilding with full dataset
- Test recovery from data loss
- Verify backup/restore procedures

### 3. Operational Requirements
- Monitor disk space for persistence layer
- Regular backup verification
- Alert on any data loss events
- Periodic data integrity checks

## Recovery Actions

### Immediate Actions
1. Ensure eviction is logged and capped by `max_races`; avoid silent loss.
2. Add snapshot/restore helpers and document procedures.
3. Add monitoring and alerts for capacity/evictions/failed flushes.

### Long-term Actions
1. Formalize data lifecycle policies and retention.
2. Harden export/import and add schema migration tooling.
3. Explore replication options if multi-host is required.
4. Add periodic audits and integrity checks with repair utilities.
