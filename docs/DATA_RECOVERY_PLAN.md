# Data Recovery and Integrity Plan

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

### Phase 2: Architectural Fixes
1. **Remove in-memory race limit** or increase to 100,000+
2. **Always persist first, memory second** - Write-through cache pattern
3. **Separate historic from active** - Different storage strategies
4. **Implement WAL (Write-Ahead Logging)** for crash recovery

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
- **Primary**: SQLite database at `~/.raceboard/races.db`
- **WAL**: Write-ahead log at `~/.raceboard/wal/`
- **Backup**: Daily snapshots at `~/.raceboard/backups/`
- **Export**: JSON export capability for external backup

### 4. Monitoring
- Storage capacity alerts
- Data loss detection
- Backup verification
- Cluster rebuild data sufficiency checks

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
1. Remove the 1000 race limit
2. Implement write-through persistence
3. Add data recovery tooling
4. Add monitoring and alerts

### Long-term Actions
1. Implement proper data lifecycle management
2. Add data export/import capabilities
3. Implement distributed storage for redundancy
4. Add data validation and integrity checks