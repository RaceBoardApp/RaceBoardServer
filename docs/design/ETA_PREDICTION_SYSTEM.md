# ETA Prediction System - Complete Documentation

## Overview

Related docs:
- Server Guide: `../guides/SERVER_GUIDE.md`
- Data layer and persistence: `../specs/DATA_LAYER_SPECIFICATION.md`
- Cluster rebuilding and rollout: `../proposals/CLUSTER_REBUILDING_PROPOSAL.md`
- HTTP API reference: `api/openapi.yaml`
- gRPC schema: `grpc/race.proto`

The Raceboard ETA prediction system uses machine learning clustering to predict execution times for races (jobs/tasks). It groups similar races together based on their characteristics and uses historical execution data to make accurate predictions for new races.

## Table of Contents

1. [Architecture](#architecture)
2. [Core Components](#core-components)
3. [Clustering Engine](#clustering-engine)
4. [Data Model](#data-model)
5. [Prediction Flow](#prediction-flow)
6. [Cluster Rebuilding](#cluster-rebuilding)
7. [Phased Rollout](#phased-rollout)
8. [Monitoring & Metrics](#monitoring--metrics)
9. [API Endpoints](#api-endpoints)
10. [Implementation Details](#implementation-details)

## Architecture

The system consists of several interconnected components:

```
┌─────────────────────────────────────────────────────────────┐
│                         Client API                          │
├─────────────────────────────────────────────────────────────┤
│                     Prediction Engine                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │   Cluster    │  │   Source     │  │   Bootstrap  │     │
│  │  Prediction  │→ │   Averages   │→ │   Defaults   │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
├─────────────────────────────────────────────────────────────┤
│                    Clustering Engine                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │ HNSW-DBSCAN  │  │   Feature    │  │  Execution   │     │
│  │  Algorithm   │  │  Extraction  │  │  Statistics  │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
├─────────────────────────────────────────────────────────────┤
│                  Rebuild & Monitoring                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │   Rebuild    │  │    Phased    │  │   Metrics    │     │
│  │   Trigger    │  │    Rollout   │  │  Collection  │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
├─────────────────────────────────────────────────────────────┤
│                    Persistence Layer                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │     Sled     │  │   Race Data  │  │   Clusters   │     │
│  │   Database   │  │   Storage    │  │   Storage    │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

## Cluster Rebuilding (merged)

This section consolidates the core of the former `../proposals/CLUSTER_REBUILDING_PROPOSAL.md` into the ETA guide so all prediction and clustering logic lives in one place.

### Triggers
- Periodic: weekly rebuild per source.
- Metric-based:
  - Mean Absolute Error (MAE) > 20% of median execution time.
  - Average cohesion < 0.7.
  - Noise ratio > 15%.

### Rebuild Algorithm (DBSCAN per source)
- Partition by `race.source`; rebuild clusters independently per source.
- Distance combines normalized Levenshtein(title) and Jaccard(metadata) via weights from `SourceConfig`.
- HNSW assists neighbor search with exact rechecks (see `src/hnsw_dbscan.rs`).

```rust
// See src/rebuild.rs and src/hnsw_dbscan.rs for the production code.
pub fn custom_distance(r1: &Race, r2: &Race, cfg: &SourceConfig) -> f64 {
    if r1.source != r2.source { return 1.0; }
    let t1 = normalize_text(&r1.title);
    let t2 = normalize_text(&r2.title);
    let max_len = t1.chars().count().max(t2.chars().count()) as f64;
    let d_title = if max_len > 0.0 { levenshtein::levenshtein(&t1, &t2) as f64 / max_len } else { 0.0 };
    let d_meta  = 1.0 - jaccard_metadata_similarity(r1.metadata.as_ref(), r2.metadata.as_ref());
    (cfg.w_title * d_title + cfg.w_meta * d_meta).clamp(0.0, 1.0)
}
```

### Parameter Tuning (Kneedle)
- Compute k-distance (k = `min_samples`) over a fixed-seed subsample.
- Smooth and detect knee with Kneedle to select `eps` within `eps_range`.
- Persist `last_eps` per source with EMA smoothing (`eps_ema_smoothing`).

### Phased Rollout
1. SingleSource (pilot) → Shadow.
2. AllSourcesConservative.
3. AutomaticTuning. Rollback on failing validation.

### Configuration (excerpt)
```rust
pub struct RebuildConfig {
  pub source_configs: HashMap<String, SourceConfig>,
  pub max_clusters: usize,
  pub max_memory_multiplier: f64,
  pub validation_holdout_ratio: f64,
  pub max_mae_increase: f64,
  pub max_noise_ratio: f64,
  pub min_cohesion: f64,
  pub min_separation: f64,
  pub min_ari: f64,
  pub use_ann_optimization: bool,
  pub distance_cache_size: usize,
  pub batch_size: usize,
  pub max_rebuild_duration: Duration,
  pub shadow_mode_duration: Duration,
  pub canary_duration: Duration,
  pub rebuild_interval: Duration,
  pub kneedle_sensitivity: f64,
  pub kneedle_smoothing: usize,
  pub metric_version: String,
  pub tokenizer_version: String,
  pub eps_ema_smoothing: f32,
}
```

For the detailed algorithms, data structures, and validation metrics, see the implementation in `src/rebuild.rs`, `src/hnsw_dbscan.rs`, and the tests under `tests/`.

## Core Components

### 1. Race Model (`src/models.rs`)

```rust
pub struct Race {
    pub id: String,
    pub source: String,              // e.g., "cargo", "npm", "claude-code"
    pub title: String,                // e.g., "cargo build --release"
    pub state: RaceState,            // Queued, Running, Passed, Failed, Canceled
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,  // Set when race completes
    pub duration_sec: Option<i64>,            // Calculated by server
    pub eta_sec: Option<i64>,                 // Predicted duration
    pub progress: Option<i32>,
    pub deeplink: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub events: Option<Vec<Event>>,
}
```

**Key Point**: The server automatically calculates `duration_sec` when a race transitions to a completed state (Passed/Failed/Canceled).

### 2. Cluster Model (`src/cluster.rs`)

```rust
pub struct RaceCluster {
    pub cluster_id: String,
    pub source: String,
    pub representative_title: String,
    pub member_race_ids: Vec<String>,
    pub stats: ExecutionStats,
    pub representative_metadata: HashMap<String, String>,
    pub last_updated: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}

pub struct ExecutionStats {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    pub std_dev: f64,
    pub min: i64,
    pub max: i64,
    pub p95: f64,
    pub p99: f64,
    pub recent_samples: VecDeque<i64>,  // Last 100 samples
}
```

## Clustering Engine

### HNSW-DBSCAN Algorithm (`src/hnsw_dbscan.rs`)

The system uses a hybrid approach combining:
- **HNSW (Hierarchical Navigable Small World)**: For fast approximate nearest neighbor search
- **DBSCAN**: For density-based clustering

#### Distance Calculation

```rust
fn calculate_distance(race1: &Race, race2: &Race) -> f32 {
    // Source must match exactly
    if race1.source != race2.source {
        return 1.0;
    }
    
    // Weighted combination of:
    // 1. Title similarity (60% weight)
    let title_sim = levenshtein_similarity(&race1.title, &race2.title);
    
    // 2. Metadata similarity (40% weight)  
    let meta_sim = jaccard_similarity(&race1.metadata, &race2.metadata);
    
    1.0 - (0.6 * title_sim + 0.4 * meta_sim)
}
```

#### Clustering Parameters

- **eps**: Dynamically determined using Kneedle algorithm on k-distance graph
- **min_samples**: Varies by source (default: 3)
- **Distance threshold**: 0.3 (races must be 70% similar)

### Feature Extraction

The system extracts features from races to enable clustering:

1. **Title Features**:
   - Tokenization and normalization
   - Command structure analysis
   - Parameter extraction

2. **Metadata Features**:
   - Key-value pairs from race metadata
   - Semantic grouping of related keys
   - Numeric value normalization

3. **Temporal Features**:
   - Time of day patterns
   - Day of week patterns
   - Execution frequency

## Data Model

### Persistence Layer (`src/persistence.rs`)

The system uses Sled embedded database with multiple trees:

1. **races_tree**: Stores all race data
2. **races_by_time**: Time-based index for efficient queries
3. **clusters_tree**: Stores cluster definitions
4. **source_stats_tree**: Aggregate statistics per source
5. **meta_tree**: System metadata and configuration

### Data Retention

- **Active races**: Kept in memory for fast access
- **Historical races**: Persisted in Sled database
- **Retention period**: 1 year (configurable)
- **Cluster data**: Rebuilt periodically, persisted between rebuilds

## Prediction Flow

### 1. New Race Creation

```
Client → POST /race → Create Race → Extract Features → Find Cluster → Return ETA
```

### 2. Prediction Algorithm

```rust
async fn predict_eta(race: &Race) -> Option<i64> {
    // Level 1: Try cluster prediction
    if let Some(cluster) = find_matching_cluster(race) {
        return Some(cluster.stats.median as i64);
    }
    
    // Level 2: Try source average
    if let Some(source_stats) = get_source_statistics(&race.source) {
        return Some(source_stats.average as i64);
    }
    
    // Level 3: Bootstrap defaults
    get_bootstrap_default(&race.source)
}
```

### 3. Fallback Strategy

1. **Cluster Match** (Primary): Use cluster's median execution time
2. **Source Average** (Secondary): Average of all races from same source
3. **Bootstrap Defaults** (Tertiary): Hard-coded defaults by source type
   - cargo: 45 seconds
   - npm: 30 seconds
   - claude-code: 60 seconds
   - Default: 30 seconds

## Cluster Rebuilding

### Rebuild Trigger (`src/rebuild_trigger.rs`)

The system automatically rebuilds clusters when:

1. **MAE Degradation**: Mean Absolute Error exceeds 20% of median execution time
2. **Scheduled**: Daily rebuild at low-traffic times
3. **Manual**: Via `/rebuild/trigger` endpoint
4. **Data Volume**: When significant new data accumulated (>1000 new races)

### Zero-Downtime Rebuild (`src/rebuild.rs`)

```rust
pub async fn rebuild_with_zero_downtime() {
    // 1. Build new clusters in background
    let new_clusters = build_clusters_from_races().await;
    
    // 2. Validate new clusters
    if !validate_clusters(&new_clusters) {
        return; // Keep existing clusters if validation fails
    }
    
    // 3. Atomic swap
    let old_clusters = self.active_clusters.swap(new_clusters);
    
    // 4. Clean up old clusters
    cleanup(old_clusters);
}
```

### Validation Criteria

- **Minimum cluster size**: 2 races
- **Maximum noise ratio**: 30%
- **Minimum ARI score**: 0.7 (Adjusted Rand Index)
- **MAE threshold**: Within 20% of actual execution times

## Phased Rollout

### Rollout Phases (`src/phased_rollout.rs`)

The system uses a gradual rollout strategy:

1. **Phase 1 - Single Source**: 
   - Enable for one pilot source (e.g., "cargo")
   - Monitor metrics for stability

2. **Phase 2 - All Sources Conservative**:
   - Enable for all sources with conservative parameters
   - Higher min_samples, tighter eps

3. **Phase 3 - Automatic Tuning**:
   - Enable dynamic parameter optimization
   - Self-adjusting based on performance

### Source Discovery

Sources are automatically discovered from the database:
- Scans all races on startup
- Registers new sources dynamically
- Persists configuration across restarts

## Monitoring & Metrics

### Storage Health (`src/monitoring.rs`)

```rust
pub struct StorageHealth {
    pub total_races: usize,
    pub races_by_source: HashMap<String, usize>,
    pub cluster_data_sufficient: bool,  // Need 1000+ races per source
    pub usage_percent: f64,
    pub eviction_count: usize,
    pub warnings: Vec<String>,
    pub critical_errors: Vec<String>,
}
```

### Data Layer Metrics

- **Gauges**:
  - `sled_db_size_bytes`: Database size
  - `tree_counts`: Entries per tree
  - `read_only_mode_active`: System status

- **Counters**:
  - `flush_failures_total`: Persistence failures
  - `serialize_failures_total`: Data corruption events
  - `json_snapshot_success_total`: Backup successes

- **Histograms**:
  - `write_latency_ms`: p50, p95, p99
  - `flush_latency_ms`: p50, p95, p99

### SLOs (Service Level Objectives)

- Write latency p95 ≤ 25ms
- Flush latency p99 ≤ 200ms
- Prediction accuracy ≥ 80% within ±20% of actual
- Cluster rebuild time ≤ 30 seconds

## API Endpoints

### Race Management

- `POST /race` - Create or update race
- `PATCH /race/:id` - Update specific fields
- `GET /race/:id` - Get single race
- `GET /races` - List all active races
- `DELETE /race/:id` - Delete race

### Historic Data

- `GET /historic/races` - Query historical races with filters
  - Query params: `source`, `from`, `to`, `limit`, `cursor`

### Clustering

- `GET /clusters` - List all clusters
- `GET /cluster/:id` - Get cluster details
- `POST /rebuild/trigger` - Manually trigger rebuild

### Admin

- `POST /admin/purge` - Remove specific races
- `POST /admin/compact` - Compact database
- `GET /admin/storage-report` - Storage statistics
- `GET /admin/metrics` - System metrics

### Monitoring

- `GET /health` - System health check
- `GET /metrics/rebuild` - Rebuild metrics
- `GET /metrics/rollout` - Rollout status

## Implementation Details

### Key Files

1. **Core Logic**:
   - `src/models.rs` - Data models
   - `src/cluster.rs` - Clustering engine
   - `src/prediction.rs` - ETA prediction
   - `src/hnsw_dbscan.rs` - HNSW-DBSCAN implementation

2. **Rebuild System**:
   - `src/rebuild.rs` - Cluster rebuilding logic
   - `src/rebuild_trigger.rs` - Automatic rebuild triggers
   - `src/phased_rollout.rs` - Gradual rollout control

3. **Storage**:
   - `src/persistence.rs` - Sled database layer
   - `src/storage.rs` - In-memory storage
   - `src/monitoring.rs` - Health monitoring

4. **API**:
   - `src/handlers.rs` - HTTP request handlers
   - `src/grpc_service.rs` - gRPC service implementation

### Configuration

```rust
pub struct ClusterConfig {
    pub eps_range: (f32, f32),        // (0.3, 0.5)
    pub min_samples: usize,           // 3
    pub min_cluster_size: usize,      // 2
    pub w_title: f32,                 // 0.6 - title weight
    pub w_meta: f32,                  // 0.4 - metadata weight
    pub tau_split: f32,               // 0.35 - split threshold
    pub tau_merge: (f32, f32),        // (0.35, 0.6) - merge range
}
```

### Bootstrap Defaults

```rust
const BOOTSTRAP_DEFAULTS: &[(&str, i64)] = &[
    ("cargo", 45),
    ("npm", 30),
    ("gradle", 60),
    ("maven", 50),
    ("make", 40),
    ("pytest", 35),
    ("go", 25),
    ("claude-code", 60),
    ("codex", 45),
    ("gemini", 50),
];
```

## Performance Characteristics

### Time Complexity

- **Prediction**: O(log N) using HNSW index
- **Cluster rebuild**: O(N log N) for N races
- **Race insertion**: O(1) amortized
- **Historic query**: O(log N + K) for K results

### Space Complexity

- **Memory**: ~1KB per active race
- **Disk**: ~2KB per persisted race
- **Clusters**: ~10KB per cluster
- **Index**: ~100 bytes per race

### Scalability

- Tested with 1M+ races
- Supports 100+ concurrent clients
- Sub-second prediction latency
- 30-second full rebuild for 100K races

## Future Enhancements

1. **Machine Learning Models**:
   - Deep learning for complex pattern recognition
   - Time series forecasting for temporal patterns
   - Ensemble methods for improved accuracy

2. **Advanced Features**:
   - Multi-stage race support
   - Dependency-aware predictions
   - Resource-based scaling factors

3. **Operational**:
   - Prometheus metrics export
   - Distributed clustering for horizontal scaling
   - Real-time cluster updates without rebuild

## Conclusion

The ETA prediction system provides accurate execution time estimates through:
- Intelligent clustering of similar races
- Multiple fallback strategies for robustness
- Automatic data-driven improvements
- Production-ready monitoring and operations

The system continuously learns from new data, improving predictions over time while maintaining sub-second response times and high availability.
