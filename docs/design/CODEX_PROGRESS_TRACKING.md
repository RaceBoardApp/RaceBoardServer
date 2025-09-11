# Codex Progress Tracking Strategy

## The Challenge

Tracking progress for AI sessions is inherently difficult because:
- We don't know how many steps the AI will take
- Task complexity varies dramatically
- AI can change plans mid-execution

## Multi-Level Progress Tracking

### 1. Plan-Based Progress (Most Accurate)

When Codex uses `update_plan`, we have explicit progress information:

```rust
struct PlanProgress {
    total_steps: usize,
    completed_steps: usize,
    current_step: Option<String>,
}

impl PlanProgress {
    fn calculate_percentage(&self) -> i32 {
        if self.total_steps == 0 {
            return 0;
        }
        ((self.completed_steps as f64 / self.total_steps as f64) * 100.0) as i32
    }
}

// Parse plan updates
fn handle_plan_update(&mut self, json_str: &str) {
    if let Ok(data) = serde_json::from_str::<Value>(json_str) {
        if let Some(plan) = data["plan"].as_array() {
            let total = plan.len();
            let completed = plan.iter()
                .filter(|step| step["status"].as_str() == Some("completed"))
                .count();
            let in_progress = plan.iter()
                .find(|step| step["status"].as_str() == Some("in_progress"));
            
            let progress = PlanProgress {
                total_steps: total,
                completed_steps: completed,
                current_step: in_progress.and_then(|s| s["step"].as_str().map(String::from)),
            };
            
            let percentage = progress.calculate_percentage();
            self.update_race_progress(percentage).await;
        }
    }
}
```

### 2. Function Call Counting (Activity-Based)

Track the rate of function calls as a proxy for activity:

```rust
struct ActivityTracker {
    start_time: Instant,
    function_call_count: u32,
    last_activity: Instant,
    estimated_total_calls: u32, // Based on historical data
}

impl ActivityTracker {
    fn estimate_progress(&self) -> i32 {
        // Method 1: Time-based (if we have ETA)
        if let Some(eta_sec) = self.initial_eta {
            let elapsed = self.start_time.elapsed().as_secs();
            let progress = ((elapsed as f64 / eta_sec as f64) * 100.0).min(95.0);
            return progress as i32;
        }
        
        // Method 2: Activity decay (no activity = nearing completion)
        let idle_time = self.last_activity.elapsed().as_secs();
        if idle_time > 10 {
            return 90; // Likely finishing up
        }
        
        // Method 3: Call count heuristic
        // Most Codex tasks involve 10-50 function calls
        let estimated_progress = match self.function_call_count {
            0..=5 => 10,   // Just starting
            6..=10 => 25,  // Early stage
            11..=20 => 50, // Mid-way
            21..=30 => 75, // Most work done
            31..=40 => 85, // Wrapping up
            _ => 90,       // Nearly done
        };
        
        estimated_progress
    }
}
```

### 3. Hybrid Approach with Learning

Combine multiple signals and learn from historical data:

```rust
struct SmartProgressTracker {
    plan_progress: Option<PlanProgress>,
    activity_tracker: ActivityTracker,
    prompt_complexity: PromptComplexity,
    historical_data: HistoricalStats,
}

#[derive(Debug)]
enum PromptComplexity {
    Simple,    // "What's up?", "List files"
    Moderate,  // "Fix this bug", "Add a feature"
    Complex,   // "Refactor the architecture", "Implement full system"
}

impl SmartProgressTracker {
    fn analyze_prompt(prompt: &str) -> PromptComplexity {
        let word_count = prompt.split_whitespace().count();
        let has_code = prompt.contains("```") || prompt.contains("code");
        let action_words = ["implement", "refactor", "design", "create", "build"];
        let has_complex_action = action_words.iter().any(|w| prompt.to_lowercase().contains(w));
        
        match (word_count, has_complex_action, has_code) {
            (0..=10, false, false) => PromptComplexity::Simple,
            (_, true, _) | (30.., _, _) => PromptComplexity::Complex,
            _ => PromptComplexity::Moderate,
        }
    }
    
    fn estimate_eta(&self) -> i64 {
        match self.prompt_complexity {
            PromptComplexity::Simple => 10,
            PromptComplexity::Moderate => 30,
            PromptComplexity::Complex => 120,
        }
    }
    
    fn calculate_progress(&self) -> i32 {
        // Priority 1: Explicit plan progress
        if let Some(ref plan) = self.plan_progress {
            return plan.calculate_percentage();
        }
        
        // Priority 2: Activity-based with complexity adjustment
        let base_progress = self.activity_tracker.estimate_progress();
        
        // Adjust based on complexity
        let adjusted = match self.prompt_complexity {
            PromptComplexity::Simple => (base_progress as f64 * 1.2).min(100.0),
            PromptComplexity::Moderate => base_progress as f64,
            PromptComplexity::Complex => (base_progress as f64 * 0.8).max(5.0),
        };
        
        adjusted as i32
    }
}
```

### 4. Visual Progress Indicators

For uncertain progress, use different visual representations:

```rust
enum ProgressDisplay {
    /// Known progress with percentage
    Determinate { percentage: i32 },
    
    /// Activity-based, no clear endpoint
    Indeterminate { active: bool },
    
    /// Stepped progress (1 of N)
    Stepped { current: usize, total: usize },
    
    /// Time-based estimate
    TimeBased { elapsed: Duration, estimated_total: Duration },
}

impl ProgressDisplay {
    fn to_race_progress(&self) -> Option<i32> {
        match self {
            ProgressDisplay::Determinate { percentage } => Some(*percentage),
            ProgressDisplay::Indeterminate { active } => {
                if *active { Some(50) } else { Some(95) }
            },
            ProgressDisplay::Stepped { current, total } => {
                if *total > 0 {
                    Some(((*current as f64 / *total as f64) * 100.0) as i32)
                } else {
                    Some(0)
                }
            },
            ProgressDisplay::TimeBased { elapsed, estimated_total } => {
                let progress = (elapsed.as_secs() as f64 / estimated_total.as_secs() as f64) * 100.0;
                Some(progress.min(95.0) as i32)
            },
        }
    }
}
```

## Implementation Example

```rust
impl CodexLogWatcher {
    async fn process_log_line(&mut self, line: &str) {
        // Track user input
        if line.contains("DEBUG Submission") && line.contains("UserInput") {
            let prompt = self.extract_user_input(line);
            let complexity = SmartProgressTracker::analyze_prompt(&prompt);
            let eta = match complexity {
                PromptComplexity::Simple => 10,
                PromptComplexity::Moderate => 30,
                PromptComplexity::Complex => 120,
            };
            
            self.current_tracker = Some(SmartProgressTracker {
                plan_progress: None,
                activity_tracker: ActivityTracker::new(eta),
                prompt_complexity: complexity,
                historical_data: self.load_historical_stats(),
            });
            
            // Create race with estimated ETA
            let race = Race {
                id: Uuid::new_v4().to_string(),
                source: "codex-session".to_string(),
                title: format!("Codex: {}", truncate(&prompt, 50)),
                state: RaceState::Running,
                started_at: Utc::now().to_rfc3339(),
                eta_sec: Some(eta),
                progress: Some(0),
                // ...
            };
        }
        
        // Update progress on function calls
        if line.contains("INFO FunctionCall") {
            if let Some(ref mut tracker) = self.current_tracker {
                tracker.activity_tracker.function_call_count += 1;
                tracker.activity_tracker.last_activity = Instant::now();
                
                // Extract function type for better estimates
                if line.contains("update_plan") {
                    // Parse plan for accurate progress
                    let plan_data = self.extract_json_from_line(line);
                    tracker.update_plan_progress(plan_data);
                }
                
                let progress = tracker.calculate_progress();
                self.update_race_progress(progress).await;
            }
        }
        
        // Complete on turn finished
        if line.contains("DEBUG Turn completed") {
            if let Some(tracker) = self.current_tracker.take() {
                // Save stats for learning
                self.save_session_stats(SessionStats {
                    prompt_complexity: tracker.prompt_complexity,
                    total_function_calls: tracker.activity_tracker.function_call_count,
                    duration: tracker.activity_tracker.start_time.elapsed(),
                });
                
                self.complete_race(100).await;
            }
        }
    }
}
```

## Progress Heuristics Summary

1. **Plan steps**: Most accurate when available (completed/total * 100)
2. **Function call rate**: Good proxy for activity level
3. **Idle detection**: No calls for 10+ seconds = likely finishing
4. **Prompt complexity**: Simple prompts complete faster
5. **Historical learning**: Use past sessions to improve estimates

## Fallback Strategy

When uncertain, use conservative estimates:
- Start at 10% immediately
- Increment by 10% every N function calls
- Jump to 90% after idle period
- Only show 100% when "Turn completed" is seen

This ensures the progress bar is always moving and provides useful feedback even without perfect information.