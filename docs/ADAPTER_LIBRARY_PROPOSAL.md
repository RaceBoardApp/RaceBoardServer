# Adapter Library Refactoring Proposal

## Executive Summary

This proposal outlines a strategy to deduplicate and simplify the Raceboard adapter codebase by extracting common functionality into a shared library. This will reduce code size by ~60%, improve maintainability, and fix reliability issues like the GitLab timeout problem.

## Current Problems

### 1. Code Duplication
- **37,059 lines** in raceboard_gitlab.rs
- **32,677 lines** in raceboard_codex_watch.rs  
- **25,674 lines** in claude_adapter.rs
- **23,710 lines** in raceboard_gemini_watch.rs
- **23,015 lines** in raceboard_calendar.rs

Total: **~142,000 lines** across adapters with ~60% duplication

### 2. Reliability Issues
- GitLab adapter hangs on network timeouts
- No consistent retry logic across adapters
- Inconsistent error handling
- Missing connection pooling

### 3. Maintenance Burden
- Bug fixes must be applied to each adapter separately
- New features require changes in multiple places
- Testing overhead for duplicate code
- Inconsistent behavior between adapters

## Proposed Solution: `raceboard-adapter-core` Library

### Architecture

```
raceboard-adapter-core/
├── src/
│   ├── lib.rs           # Public API
│   ├── client.rs        # RaceboardClient with retry logic
│   ├── models.rs        # Common Race, Event, State types
│   ├── config.rs        # Config loading and validation
│   ├── cli.rs           # Common CLI arguments
│   ├── progress.rs      # Progress tracking utilities
│   ├── health.rs        # Health monitoring
│   └── signals.rs       # Signal handling
└── Cargo.toml
```

### Core Components

#### 1. **RaceboardClient** - Robust HTTP Client
```rust
pub struct RaceboardClient {
    // Connection pooling
    // Automatic retries with exponential backoff
    // Request timeouts (30s default)
    // Circuit breaker for server issues
}

impl RaceboardClient {
    pub async fn create_race(&self, race: &Race) -> Result<Race>
    pub async fn update_race(&self, id: &str, update: &RaceUpdate) -> Result<()>
    pub async fn add_event(&self, race_id: &str, event: &Event) -> Result<()>
    pub async fn health_check(&self) -> Result<bool>
}
```

#### 2. **Common Models** - Shared Data Types
```rust
// Single source of truth for all adapters
pub struct Race { ... }
pub struct RaceUpdate { ... }
pub enum RaceState { ... }
pub struct Event { ... }
```

#### 3. **Configuration Framework**
```rust
pub trait AdapterConfig {
    fn server_config(&self) -> &ServerConfig;
    fn validate(&self) -> Result<()>;
}

// Automatic config discovery:
// 1. --config flag
// 2. Environment variable
// 3. Default locations
pub fn load_config<T: AdapterConfig>(args: &CommonArgs) -> Result<T>
```

#### 4. **CLI Standardization**
```rust
#[derive(Parser)]
pub struct CommonArgs {
    #[arg(long)] config: Option<PathBuf>,
    #[arg(long)] server: Option<String>,
    #[arg(long)] log_level: String,
    #[arg(long)] health_port: Option<u16>,
}
```

#### 5. **Progress Tracking**
```rust
pub struct ProgressTracker {
    // Automatic ETA calculation
    // Progress percentage
    // Step tracking
}
```

## Implementation Plan

### Phase 1: Create Core Library (Week 1)
- [x] Create `src/adapter_common.rs` with core components
- [ ] Add comprehensive tests
- [ ] Document public API
- [ ] Add examples

### Phase 2: Migrate Simple Adapters (Week 2)
- [ ] Refactor `raceboard-track` (simplest)
- [ ] Refactor `raceboard-cmd`
- [ ] Validate functionality

### Phase 3: Migrate Complex Adapters (Week 3)
- [ ] Refactor `raceboard-gitlab` with timeout fixes
- [ ] Refactor `raceboard-calendar`
- [ ] Refactor `raceboard-codex-watch`

### Phase 4: Testing & Optimization (Week 4)
- [ ] Integration tests
- [ ] Performance benchmarks
- [ ] Binary size optimization
- [ ] Documentation

## Benefits

### 1. **Code Reduction**
- Before: ~142,000 lines
- After: ~57,000 lines (~60% reduction)
- Smaller binaries (3-5 MB each vs 6-11 MB)

### 2. **Reliability Improvements**
- Consistent timeout handling (fixes GitLab hang)
- Automatic retries with backoff
- Connection pooling
- Circuit breaker pattern

### 3. **Maintainability**
- Single place for bug fixes
- Consistent behavior across adapters
- Easier to add new adapters
- Reduced testing overhead

### 4. **Performance**
- Connection reuse via pooling
- Optimized retry logic
- Parallel request capability
- Reduced memory usage

## Example: Refactored GitLab Adapter

### Before (1,000+ lines for basic operations)
```rust
// Lots of duplicate code for HTTP, config, models, etc.
struct GitLabAdapter {
    client: reqwest::Client,
    // ... manual retry logic
    // ... manual timeout handling
}
```

### After (~300 lines)
```rust
use raceboard_adapter_core::*;

#[derive(Deserialize)]
struct GitLabConfig {
    #[serde(flatten)]
    server: ServerConfig,
    gitlab: GitLabSettings,
}

struct GitLabAdapter {
    client: RaceboardClient,  // All HTTP logic handled
    gitlab: GitLabApi,
}

impl GitLabAdapter {
    async fn sync_pipelines(&self) -> Result<()> {
        // Focus only on GitLab-specific logic
        let pipelines = self.gitlab.fetch_pipelines().await?;
        
        for pipeline in pipelines {
            let race = self.convert_to_race(pipeline);
            self.client.create_race(&race).await?;  // Automatic retry
        }
        Ok(())
    }
}
```

## Migration Guide

### For Each Adapter:

1. **Add Dependency**
```toml
[dependencies]
raceboard-adapter-core = { path = "../adapter-core" }
```

2. **Remove Duplicate Code**
- Delete local Race/Event structs
- Delete HTTP client code
- Delete config loading boilerplate
- Delete signal handling

3. **Use Core Components**
```rust
use raceboard_adapter_core::{
    RaceboardClient, Race, RaceState, 
    CommonArgs, load_config, shutdown_signal
};
```

4. **Focus on Adapter Logic**
- Keep only adapter-specific code
- Use provided client for all API calls
- Leverage progress tracker for updates

## Testing Strategy

### Unit Tests
- Test each core component in isolation
- Mock HTTP responses
- Test retry logic with failures

### Integration Tests
- Test real adapter scenarios
- Verify backward compatibility
- Load testing with connection pools

### Regression Tests
- Ensure all existing functionality works
- Verify config file compatibility
- Check CLI argument parsing

## Rollout Plan

1. **Deploy core library** without changing adapters
2. **Migrate one adapter** as proof of concept
3. **A/B test** old vs new adapter
4. **Progressive rollout** of remaining adapters
5. **Deprecate old code** after validation

## Success Metrics

- [ ] 60% code reduction achieved
- [ ] Zero timeout hangs in GitLab adapter
- [ ] All adapters pass existing tests
- [ ] Binary size reduced by 40%
- [ ] 90% test coverage on core library
- [ ] Documentation complete

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Breaking changes | High | Extensive testing, gradual rollout |
| Performance regression | Medium | Benchmark before/after |
| Config incompatibility | Low | Support legacy format |
| Missing edge cases | Medium | Keep old code during transition |

## Timeline

- **Week 1**: Core library development
- **Week 2**: Simple adapter migration
- **Week 3**: Complex adapter migration  
- **Week 4**: Testing and optimization
- **Week 5**: Documentation and rollout

## Conclusion

This refactoring will significantly improve the Raceboard adapter ecosystem by:
- Reducing code duplication by 60%
- Fixing reliability issues like timeouts
- Improving maintainability
- Enabling faster development of new adapters

The shared library approach is a proven pattern that will make the codebase more professional, reliable, and easier to work with.