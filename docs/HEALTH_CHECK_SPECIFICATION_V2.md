# Health Check System Specification v2 - Addressing Critical Gaps

## 1. Identity Model - Supporting Multiple Instances

### 1.1 Two-Level Identity System

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct AdapterIdentity {
    // Type of adapter (gitlab, calendar, etc.)
    pub adapter_type: String,      // e.g., "gitlab"
    
    // Unique instance identifier
    pub instance_id: String,        // e.g., "gitlab-prod-1", "gitlab-dev", auto-generated UUID
    
    // Composite key for uniqueness
    pub registration_key: String,   // "{adapter_type}:{instance_id}"
}

// Example: Multiple GitLab instances
// - gitlab:gitlab-prod-1 (monitoring production)
// - gitlab:gitlab-dev-1 (monitoring development)
// - gitlab:gitlab-personal (monitoring personal projects)
```

### 1.2 Instance Registration

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRegistration {
    // Identity
    pub adapter_type: String,
    pub instance_id: String,        // Can be auto-generated if not provided
    pub display_name: String,       // e.g., "GitLab CI (Production)"
    
    // Instance-specific configuration
    pub instance_config: InstanceConfig,
    
    // Process information (for App Store edition)
    pub process_info: Option<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub config_path: PathBuf,       // Instance-specific config file
    pub working_directory: PathBuf,
    pub environment: HashMap<String, String>,
    pub command_args: Vec<String>,  // Instance-specific CLI args
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,    // Login Item Helper PID
    pub launch_token: String,        // Token from Login Item Helper
    pub started_by: String,          // "login-item-helper" or "manual"
}
```

## 2. Precise State Model with Time Bounds

### 2.1 Unambiguous State Definitions

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AdapterState {
    // Initial state (T+0)
    Initializing,    // Just registered, awaiting first health report
                    // → Timeout: 30 seconds → TimedOut
    
    // Operational states
    Running,        // Receiving health reports, all metrics normal
                   // Must receive report within: report_interval + 5s
    
    Warning,        // Receiving reports, but with warnings (high CPU, etc.)
                   // Same timing as Running
    
    Critical,       // Receiving reports, but with critical issues
                   // Same timing as Running
    
    // Communication issues (precise time bounds)
    Delayed,        // Missed 1 report (T > report_interval + 5s)
                   // → Next timeout: +report_interval → Absent
    
    Absent,         // Missed 2-3 reports (T > 2×report_interval + 5s)
                   // → Next timeout: +report_interval → Abandoned
    
    // Terminal states
    Abandoned,      // Missed 4+ reports (T > 3×report_interval + 5s)
                   // Requires re-registration to recover
    
    Terminated,     // Gracefully shut down with deregistration
    
    TimedOut,       // Failed to send first health report after registration
    
    // Special state for non-reporting adapters
    Exempt,         // Claude and CMD - no health checks expected
}

// Time-based transition rules (enforced by state machine)
pub struct StateTransitionRules {
    pub max_initialization_time: Duration,     // 30 seconds
    pub report_grace_period: Duration,         // 5 seconds
    pub delayed_threshold: u32,                // 1 missed report
    pub absent_threshold: u32,                 // 2 missed reports
    pub abandoned_threshold: u32,              // 4 missed reports
}
```

### 2.2 State Machine Implementation

```rust
impl AdapterState {
    /// Calculate next state based on time since last report
    pub fn calculate_next_state(
        current: AdapterState,
        time_since_last_report: Duration,
        expected_interval: Duration,
        report_received: bool,
    ) -> AdapterState {
        let grace = Duration::from_secs(5);
        
        if report_received {
            // Any state can recover to Running with a good report
            return AdapterState::Running;
        }
        
        match current {
            AdapterState::Initializing => {
                if time_since_last_report > Duration::from_secs(30) {
                    AdapterState::TimedOut
                } else {
                    current
                }
            }
            AdapterState::Running | AdapterState::Warning | AdapterState::Critical => {
                if time_since_last_report > expected_interval + grace {
                    AdapterState::Delayed
                } else {
                    current
                }
            }
            AdapterState::Delayed => {
                if time_since_last_report > (expected_interval * 2) + grace {
                    AdapterState::Absent
                } else {
                    current
                }
            }
            AdapterState::Absent => {
                if time_since_last_report > (expected_interval * 3) + grace {
                    AdapterState::Abandoned
                } else {
                    current
                }
            }
            _ => current,  // Terminal states don't auto-transition
        }
    }
}
```

## 3. Complete API Contracts with OpenAPI

### 3.1 OpenAPI Specification

```yaml
openapi: 3.0.3
info:
  title: Raceboard Adapter Health API
  version: 1.0.0
  
servers:
  - url: http://127.0.0.1:7777/api/v1
    description: Local-only adapter API

security:
  - BearerAuth: []  # Token from Login Item Helper
  
paths:
  /adapter/register:
    post:
      summary: Register adapter instance
      security:
        - BearerAuth: []
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/RegisterRequest'
            examples:
              gitlab_prod:
                value:
                  adapter_type: "gitlab"
                  instance_id: "gitlab-prod-1"
                  display_name: "GitLab CI (Production)"
                  capabilities:
                    supports_health_check: true
                    health_interval_seconds: 30
      responses:
        '201':
          description: Successfully registered
          headers:
            X-Registration-Token:
              schema:
                type: string
              description: Token for subsequent health reports
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/RegisterResponse'
        '400':
          $ref: '#/components/responses/BadRequest'
        '401':
          $ref: '#/components/responses/Unauthorized'
        '409':
          $ref: '#/components/responses/Conflict'
        '429':
          $ref: '#/components/responses/RateLimited'
          
components:
  securitySchemes:
    BearerAuth:
      type: http
      scheme: bearer
      description: Token issued by Login Item Helper
      
  responses:
    BadRequest:
      description: Invalid request data
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/ErrorResponse'
          example:
            error: "INVALID_INSTANCE_ID"
            message: "Instance ID contains invalid characters"
            
    Unauthorized:
      description: Invalid or missing authentication token
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/ErrorResponse'
          example:
            error: "INVALID_TOKEN"
            message: "Token not issued by Login Item Helper"
            
    RateLimited:
      description: Too many requests
      headers:
        X-RateLimit-Limit:
          schema:
            type: integer
        X-RateLimit-Remaining:
          schema:
            type: integer
        X-RateLimit-Reset:
          schema:
            type: integer
```

### 3.2 Error Codes and Limits

```rust
#[derive(Debug, Serialize, Deserialize)]
pub enum ApiError {
    // Registration errors
    InvalidInstanceId,       // Instance ID format invalid
    DuplicateRegistration,   // Instance already registered
    TooManyInstances,       // Exceeded max instances per adapter type (10)
    
    // Authentication errors
    MissingToken,           // No bearer token provided
    InvalidToken,           // Token not from Login Item Helper
    ExpiredToken,           // Token older than 24 hours
    
    // Rate limiting
    RegistrationRateLimit,  // Max 10 registrations/minute
    HealthReportRateLimit,  // Max 120 reports/minute per instance
    
    // Validation errors
    InvalidInterval,        // Health interval < 10s or > 300s
    InvalidMetrics,         // Metrics out of valid ranges
    
    // System errors
    StorageFull,           // Registry at capacity (100 instances)
    MaintenanceMode,       // System in maintenance
}

pub struct ApiLimits {
    pub max_instances_per_type: usize,           // 10
    pub max_total_instances: usize,              // 100
    pub min_health_interval_seconds: u32,        // 10
    pub max_health_interval_seconds: u32,        // 300
    pub max_request_size_bytes: usize,           // 64KB
    pub max_display_name_length: usize,          // 100
    pub max_instance_id_length: usize,           // 50
    pub token_ttl_seconds: u64,                  // 86400 (24 hours)
}
```

## 4. App Store Edition Integration

### 4.1 Login Item Helper Integration

```rust
// Login Item Helper manages adapter lifecycle
pub struct LoginItemHelper {
    // Process supervision
    managed_processes: HashMap<String, ManagedProcess>,
    
    // Token management for adapters
    issued_tokens: HashMap<String, IssuedToken>,
    
    // Communication with main app
    app_group_path: PathBuf,  // ~/Library/Group Containers/<teamid>.raceboard/
}

pub struct ManagedProcess {
    pub adapter_type: String,
    pub instance_id: String,
    pub pid: u32,
    pub launch_token: String,      // Unique token for this process
    pub started_at: DateTime<Utc>,
    pub config_path: PathBuf,
    pub restart_count: u32,
    pub last_restart: Option<DateTime<Utc>>,
}

pub struct IssuedToken {
    pub token: String,
    pub issued_to: String,          // instance_id
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub capabilities: Vec<String>,  // ["register", "report_health"]
}

impl LoginItemHelper {
    /// Launch adapter with authentication token
    pub async fn launch_adapter(
        &mut self,
        adapter_type: &str,
        instance_id: &str,
        config: &InstanceConfig,
    ) -> Result<ManagedProcess> {
        // Generate unique token for this instance
        let token = self.generate_token(instance_id);
        
        // Launch process with token in environment
        let mut cmd = Command::new(&config.binary_path);
        cmd.env("RACEBOARD_INSTANCE_ID", instance_id);
        cmd.env("RACEBOARD_AUTH_TOKEN", &token);
        cmd.env("RACEBOARD_SERVER", "http://127.0.0.1:7777");
        cmd.args(&config.command_args);
        
        let child = cmd.spawn()?;
        
        // Track the process
        let process = ManagedProcess {
            adapter_type: adapter_type.to_string(),
            instance_id: instance_id.to_string(),
            pid: child.id(),
            launch_token: token.clone(),
            started_at: Utc::now(),
            config_path: config.config_path.clone(),
            restart_count: 0,
            last_restart: None,
        };
        
        self.managed_processes.insert(instance_id.to_string(), process.clone());
        Ok(process)
    }
    
    /// Validate token from adapter
    pub fn validate_token(&self, token: &str) -> Result<String> {
        self.issued_tokens
            .iter()
            .find(|(_, t)| t.token == token && t.expires_at > Utc::now())
            .map(|(instance_id, _)| instance_id.clone())
            .ok_or_else(|| Error::InvalidToken)
    }
}
```

### 4.2 Server Token Validation

```rust
// Server validates tokens from Login Item Helper
impl AdapterRegistry {
    pub async fn validate_registration(
        &self,
        request: &RegisterRequest,
        token: &str,
    ) -> Result<()> {
        // For App Store edition, validate token with Login Item Helper
        if self.config.app_store_edition {
            let helper_socket = self.connect_to_helper().await?;
            let validation = helper_socket.validate_token(token).await?;
            
            if validation.instance_id != request.instance_id {
                return Err(Error::TokenMismatch);
            }
            
            if validation.expired {
                return Err(Error::ExpiredToken);
            }
        }
        
        Ok(())
    }
}
```

## 5. Persistence and Recovery

### 5.1 Persistence Strategy

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct PersistenceConfig {
    // Storage backend
    pub backend: PersistenceBackend,
    
    // What to persist
    pub persist_registrations: bool,        // true
    pub persist_health_history: bool,       // false (too much data)
    pub persist_metrics: bool,              // true (last value only)
    
    // Recovery behavior
    pub recovery_mode: RecoveryMode,
    
    // Cleanup
    pub ttl_abandoned_instances: Duration,  // 24 hours
    pub ttl_terminated_instances: Duration, // 1 hour
}

#[derive(Debug, Serialize, Deserialize)]
pub enum PersistenceBackend {
    // Ephemeral - registrations lost on restart
    Memory,
    
    // Persistent - survives restarts
    Sled { path: PathBuf },        // Same DB as races
    File { path: PathBuf },        // JSON file in app group
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RecoveryMode {
    // Clear all registrations on startup
    ClearOnStartup,
    
    // Mark all as Abandoned, wait for re-registration
    MarkAbandoned,
    
    // Optimistic - assume Running, verify with health checks
    OptimisticRecovery { grace_period: Duration },
    
    // App Store edition - query Login Item Helper
    QueryLoginItemHelper,
}
```

### 5.2 Recovery Implementation

```rust
impl AdapterRegistry {
    pub async fn recover_from_persistence(&mut self) -> Result<()> {
        match self.config.recovery_mode {
            RecoveryMode::ClearOnStartup => {
                // Start fresh
                self.clear_all_registrations().await?;
            }
            
            RecoveryMode::MarkAbandoned => {
                // Mark all persisted registrations as Abandoned
                for mut registration in self.load_registrations().await? {
                    registration.state = AdapterState::Abandoned;
                    registration.state_changed_at = Utc::now();
                    self.registrations.insert(
                        registration.registration_key.clone(),
                        registration,
                    );
                }
            }
            
            RecoveryMode::OptimisticRecovery { grace_period } => {
                // Assume Running, give grace period for health reports
                for mut registration in self.load_registrations().await? {
                    registration.state = AdapterState::Running;
                    registration.last_health_report_at = Some(Utc::now());
                    registration.recovery_deadline = Some(Utc::now() + grace_period);
                    self.registrations.insert(
                        registration.registration_key.clone(),
                        registration,
                    );
                }
            }
            
            RecoveryMode::QueryLoginItemHelper => {
                // App Store edition - get actual process state
                let helper = self.connect_to_helper().await?;
                let running_processes = helper.list_managed_processes().await?;
                
                for process in running_processes {
                    // Restore registration for running processes
                    let registration = Registration {
                        adapter_type: process.adapter_type,
                        instance_id: process.instance_id,
                        state: AdapterState::Running,
                        pid: Some(process.pid),
                        ..Default::default()
                    };
                    self.registrations.insert(
                        registration.registration_key.clone(),
                        registration,
                    );
                }
            }
        }
        
        Ok(())
    }
    
    pub async fn persist_current_state(&self) -> Result<()> {
        match &self.config.backend {
            PersistenceBackend::Sled { path } => {
                let db = sled::open(path)?;
                let tree = db.open_tree("adapter_registry")?;
                
                for (key, registration) in &self.registrations {
                    let data = bincode::serialize(registration)?;
                    tree.insert(key.as_bytes(), data)?;
                }
                
                tree.flush_async().await?;
            }
            
            PersistenceBackend::File { path } => {
                let data = serde_json::to_string_pretty(&self.registrations)?;
                tokio::fs::write(path, data).await?;
            }
            
            PersistenceBackend::Memory => {
                // No persistence
            }
        }
        
        Ok(())
    }
}
```

## 6. Complete Example Configuration

```toml
# Server configuration for App Store edition
[adapter_health]
enabled = true

# Persistence
backend = "sled"
persist_path = "~/Library/Group Containers/<teamid>.raceboard/adapter_registry.db"
recovery_mode = "QueryLoginItemHelper"

# Limits
max_instances_per_type = 10
max_total_instances = 100
min_health_interval_seconds = 10
max_health_interval_seconds = 300

# Security
require_auth_token = true
validate_with_helper = true
helper_socket_path = "~/Library/Group Containers/<teamid>.raceboard/helper.sock"

# Cleanup
ttl_abandoned_hours = 24
ttl_terminated_hours = 1
cleanup_interval_minutes = 60
```

## 7. Migration Path

### Phase 1: Core Identity Model
- Implement instance_id support
- Update registration to use composite keys
- Add multiple instance validation

### Phase 2: Precise State Model  
- Implement new state enum with time bounds
- Add state transition validation
- Create time-based state updater

### Phase 3: App Store Integration
- Add token validation middleware
- Implement Login Item Helper communication
- Add process supervision integration

### Phase 4: Persistence Layer
- Implement sled backend for registry
- Add recovery modes
- Create cleanup jobs

### Phase 5: OpenAPI Documentation
- Generate OpenAPI spec from code
- Add to server endpoints
- Create client SDKs

This revised specification addresses all the identified gaps with concrete, implementable solutions.