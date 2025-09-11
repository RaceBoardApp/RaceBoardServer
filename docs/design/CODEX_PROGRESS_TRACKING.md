# Codex Progress Tracking Strategy (Condensed)

This document summarizes how progress is inferred for AI coding sessions (Codex/Gemini). Detailed implementation lives in each adapter document.

## Signals (priority order)
- Plan steps (from `update_plan`) → explicit fraction = completed/total.
- Time vs ETA hint → elapsed/eta, capped at ≤95%.
- Activity signals → function-call rate and idle time.

## Heuristics
- If plan present, use it; otherwise fall back to activity/time.
- Consider idle >10s as “wrapping up” (≈90%).
- Never regress visible progress; optimistic overlay fades when server updates arrive.

## Complexity Buckets
- Simple: 10s ETA baseline; Moderate: 30s; Complex: 120s. Buckets are derived from prompt length and action words.

## Learning Hooks
- Persist session stats to refine bucket ETAs over time (source stats).
- Feed completed sessions to clustering to inform future bootstrap defaults.

## See Also
- Codex Watch adapter: `../adapters/CODEX_WATCH_ADAPTER.md`
- Gemini Watch adapter: `../adapters/GEMINI_WATCH_ADAPTER.md`

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
