# Google Calendar Adapter for Raceboard (v2)

Related docs:
- Server Guide: `docs/SERVER_GUIDE.md`
- Configuration (ICS or Google OAuth): `docs/CONFIGURATION.md`
  - ICS mode requires only your secret iCal URL (no Google Cloud)
  - Google mode uses OAuth Desktop credentials (more real‑time, less delay)

This document outlines how to implement a Google Calendar adapter that tracks calendar events as races in Raceboard, providing real-time visibility into meetings, appointments, and scheduled tasks.

## 🎯 Concept

This adapter surfaces “free‑time” as races so you can focus and see exactly how much time remains before your next meeting. It aligns with the two‑plane server model:
- When a free window starts (within working hours or during Focus Time), start a race in "running" state.
- Set ETA to the time remaining until the next meeting (or end of the free window).
- Finish the race exactly when the next meeting starts → "passed" state.
- While you are in a meeting, no free‑time race is active; the next free window will trigger the next race.

## 📋 Use Cases

1. I want focus on my work and see how much time I have before the next meeting.
2. I want to compare ETAs of current races and see whether I have a chance to do another iteration or it’s better to grab a tea before the meeting.

Based on these use cases, this adapter creates a “free‑time” race when you are free in your calendar (working hours only, or during explicit Focus Time blocks) and finishes that race exactly when the next meeting starts. The ETA shows the time remaining until the next meeting.

## 🏗️ Architecture

### Components

```
┌─────────────────────────────────────────────────────────┐
│                   Google Calendar API                     │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│              raceboard-calendar Binary                    │
│                                                           │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────┐   │
│  │   OAuth     │  │   Calendar   │  │    Race      │   │
│  │   Handler   │  │   Watcher    │  │   Manager    │   │
│  └─────────────┘  └──────────────┘  └──────────────┘   │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                  Raceboard Server API                     │
└─────────────────────────────────────────────────────────┘
```

### Data Flow (Free‑Time Races)

1. **Authentication**: OAuth2 flow to get Google Calendar access
2. **Event Polling**: Fetch upcoming events (or receive push notifications) within your working hours window
3. **Free Slot Detection**: Compute current/next free window excluding meetings (and include Focus Time blocks as free)
4. **Race Creation**: When a free window starts (or if one is already in progress), POST a race with `eta_sec` set to the time until the next meeting
5. **Progress Updates**: Periodically PATCH time‑based progress (elapsed/free‑window)
6. **Completion**: At the moment the next meeting starts, PATCH terminal state; server persists it to history

### API Contracts (Adapters)

- POST `/race`
  - id: stable id recommended (e.g., `gcal:free:<window_start>-<window_end>` or `gcal:<calendar_id>:<event_id>`)
  - source: `google-calendar`
  - title: `Until next meeting: <next_meeting_title>` (fallback: `Free time`)
  - state: `running` (free‑time race starts immediately at window start)
  - started_at: free‑window start (RFC3339, with TZ)
  - eta_sec: time remaining until next meeting (window_end - now)
  - progress: clock‑based 0..100 (elapsed/window_duration)
  - deeplink: optional (e.g., link to next meeting)
  - metadata (see schema below)

- PATCH `/race/{id}`
  - Update `progress` periodically; adjust `state` as the window elapses
  - On next meeting start → set `state=passed`, `progress=100`; do NOT set metadata (server preserves creation metadata and records completion time)

- POST `/race/{id}/event` (optional)
  - Use for reminder/alerts or attendee changes if you want rich timeline

- Read‑only mode handling
  - If the server responds 503 with header `X-Raceboard-Read-Only: 1`, back off and retry later; do not crash your adapter.

## 📦 Implementation Plan (Free‑Time Windows)

### Phase 1: Basic Implementation

```rust
// src/bin/calendar_adapter.rs (simplified)

use google_calendar3::{CalendarHub, oauth2, hyper, hyper_rustls};
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use tokio::time::interval;

#[derive(Debug, Serialize, Deserialize)]
struct Race {
    id: String,
    source: String,
    title: String,
    state: String,
    started_at: String,
    eta_sec: Option<i64>,
    progress: Option<i32>,
    deeplink: Option<String>,
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

struct CalendarAdapter {
    hub: CalendarHub,
    raceboard_client: Client,
    server_url: String,
}

impl CalendarAdapter {
    async fn new(credentials_path: &str, server_url: String) -> Result<Self> {
        // Initialize Google Calendar API client
        let secret = oauth2::read_application_secret(credentials_path)
            .await?;
        
        let auth = oauth2::InstalledFlowAuthenticator::builder(
            secret,
            oauth2::InstalledFlowReturnMethod::HTTPRedirect,
        ).persist_tokens_to_disk("calendar_tokens.json")
          .build()
          .await?;
        
        let hub = CalendarHub::new(
            hyper::Client::builder().build(
                hyper_rustls::HttpsConnectorBuilder::new()
                    .with_native_roots()
                    .https_or_http()
                    .enable_http1()
                    .build()
            ),
            auth,
        );
        
        Ok(Self {
            hub,
            raceboard_client: Client::new(),
            server_url,
        })
    }
    
    async fn watch_calendar(&self) -> Result<()> {
        let mut ticker = interval(Duration::seconds(30).to_std()?);
        loop {
            ticker.tick().await;
            self.sync_free_windows().await?;
        }
    }
    
    // NOTE: See the full free‑time sync implementation below. The previous
    // meeting‑based process_event approach has been removed to avoid confusion.
    
    fn extract_meeting_link(&self, event: &Event) -> Option<String> {
        // Check for Google Meet link
        if let Some(conference_data) = &event.conference_data {
            if let Some(entry_points) = &conference_data.entry_points {
                for entry in entry_points {
                    if entry.entry_point_type == Some("video".to_string()) {
                        return entry.uri.clone();
                    }
                }
            }
        }
        
        // Check description for Zoom/Teams links
        if let Some(description) = &event.description {
            if let Some(zoom_link) = self.extract_zoom_link(description) {
                return Some(zoom_link);
            }
            if let Some(teams_link) = self.extract_teams_link(description) {
                return Some(teams_link);
            }
        }
        
        None
    }
}
```

Add a helper to build rich metadata (prompt-like clustering fields for calendar):

```rust
impl CalendarAdapter {
    fn build_metadata(
        &self,
        calendar_id: &str,
        event: &Event,
        title: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        meeting_link: Option<String>,
    ) -> serde_json::Map<String, serde_json::Value> {
        use serde_json::json;
        let mut m = serde_json::Map::new();
        m.insert("calendar_id".into(), json!(calendar_id));
        m.insert("event_id".into(), json!(event.id.clone().unwrap_or_default()));
        m.insert("recurring_id".into(), json!(event.recurring_event_id.clone()));
        m.insert("end_time".into(), json!(end_time.to_rfc3339()));
        m.insert("timezone".into(), json!(event.start.as_ref().and_then(|s| s.time_zone.clone())));
        if let Some(link) = meeting_link { m.insert("meeting_link".into(), json!(link)); }
        if let Some(location) = &event.location { m.insert("location".into(), json!(location)); }
        let attendees = event.attendees.as_ref().map(|a|
            a.iter().filter_map(|p| p.email.clone()).collect::<Vec<_>>()
        ).unwrap_or_default();
        m.insert("attendees".into(), json!(attendees));
        if let Some(org) = event.organizer.as_ref().and_then(|o| o.email.clone()) { m.insert("organizer".into(), json!(org)); }
        m.insert("visibility".into(), json!(event.visibility.clone()));
        m.insert("reminders_overridden".into(), json!(event.reminders.as_ref().and_then(|r| r.overrides.as_ref().map(|o| !o.is_empty()))));
        let desc = event.description.clone().unwrap_or_default();
        let content = format!("{}\n{}", title, desc);
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        m.insert("event_hash".into(), json!(hash));
        let lower = title.to_lowercase();
        let cat = if lower.contains("1:1") || lower.contains("one-on-one") { "one_on_one" }
                  else if lower.contains("stand") || lower.contains("sync") { "standup" }
                  else if lower.contains("interview") { "interview" }
                  else if event.attendees.as_ref().map_or(0, |a| a.len()) > 5 { "large_meeting" }
                  else { "regular" };
        m.insert("event_category".into(), json!(cat));
        m
    }
}
```

### Phase 2: Advanced Features

```rust
// Additional features to implement

impl CalendarAdapter {
    // Real-time notifications using Google Calendar push notifications
    async fn setup_webhook(&self, webhook_url: &str) -> Result<()> {
        let channel = Channel {
            id: Some(Uuid::new_v4().to_string()),
            type_: Some("web_hook".to_string()),
            address: Some(webhook_url.to_string()),
            ..Default::default()
        };
        
        self.hub.events()
            .watch(channel, "primary")
            .doit()
            .await?;
        
        Ok(())
    }
    
    // Handle different event types
    async fn categorize_event(&self, event: &Event) -> EventCategory {
        // Detect event type based on patterns
        let title = event.summary.as_deref().unwrap_or("");
        
        if title.contains("1:1") || title.contains("One-on-One") {
            EventCategory::OneOnOne
        } else if title.contains("Stand") || title.contains("Sync") {
            EventCategory::Standup
        } else if title.contains("Interview") {
            EventCategory::Interview
        } else if event.attendees.as_ref().map_or(0, |a| a.len()) > 5 {
            EventCategory::LargeMeeting
        } else {
            EventCategory::Regular
        }
    }
    
    // Smart reminders
    async fn send_reminder(&self, event: &Event, minutes_before: i64) {
        let reminder_time = event.start_time - Duration::minutes(minutes_before);
        
        tokio::spawn(async move {
            let delay = (reminder_time - Utc::now()).to_std().unwrap();
            sleep(delay).await;
            
            // Create a "reminder" race
            let race = Race {
                id: format!("gcal-reminder-{}", event.id),
                source: "google-calendar".to_string(),
                title: format!("⏰ {} starting in {} minutes", event.title, minutes_before),
                state: "running".to_string(),
                // ... other fields
            };
            
            // This race auto-completes after 1 minute
        });
    }
}
```

## 🔧 Configuration

### OAuth Setup

1. **Enable Google Calendar API**:
   ```bash
   # Go to Google Cloud Console
   # Create a new project or select existing
   # Enable Google Calendar API
   # Create OAuth 2.0 credentials (Desktop application)
   # Download credentials.json
   ```

2. **Configuration File** (`calendar_config.toml`):
  ```toml
  [google]
  credentials_path = "credentials.json"
  token_cache = "calendar_tokens.json"
   
   [raceboard]
   server_url = "http://localhost:7777"
   
   [sync]
   interval_seconds = 30
   lookahead_hours = 24
   
   [reminders]
   enabled = true
   minutes_before = [5, 15]
   
  [filters]
  # Optional: Only track certain calendars or event types
  calendars = ["primary", "work@example.com"]
  ignore_all_day_events = true
  min_duration_minutes = 15

  [working_hours]
  enabled = true
  # Defaults if not set: 10:00–18:00 local time
  start = "10:00"    # local time (HH:MM)
  end   = "18:00"

  [focus_time]
  # Treat focus blocks as free windows inside working hours
  enabled = true
  # When event.eventType is not exposed, fallback to title patterns
  title_patterns = ["Focus", "Deep work"]
  ```

### Robustness & Best Practices
- Use stable ids to make creates idempotent and safe to retry.
- Do not overwrite metadata on completion — let the server record completion times.
- Handle read‑only mode (503 + `X-Raceboard-Read-Only: 1`) with backoff.
- Keep metadata concise; include `event_hash` for clustering. Avoid large free‑text fields.
- Progress update cadence: 30–60s is sufficient.
- Titles: use the event summary; avoid decorative prefixes/emojis.

## 🚀 Usage

### Running the Adapter

```bash
# Build the adapter
cargo build --bin raceboard-calendar

# First run - authenticate with Google
./target/debug/raceboard-calendar auth

# Run the adapter
./target/debug/raceboard-calendar watch

# Or run with custom config
./target/debug/raceboard-calendar watch --config calendar_config.toml
```

### Systemd Service (Linux/macOS)

```ini
[Unit]
Description=Raceboard Google Calendar Adapter
After=network.target

[Service]
Type=simple
User=youruser
WorkingDirectory=/path/to/raceboard
ExecStart=/path/to/raceboard-calendar watch
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

### Docker Container

```dockerfile
FROM rust:1.70 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin raceboard-calendar

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /app/target/release/raceboard-calendar /usr/local/bin/
COPY calendar_config.toml /etc/raceboard/
CMD ["raceboard-calendar", "watch", "--config", "/etc/raceboard/calendar_config.toml"]
```

## 📊 Race Data Structure

Calendar events create races with this structure:

```json
{
  "id": "gcal-abc123xyz",
  "source": "google-calendar",
  "title": "📅 Team Standup",
  "state": "running",
  "started_at": "2025-08-28T09:00:00Z",
  "eta_sec": 1800,
  "progress": 33,
  "deeplink": "https://meet.google.com/abc-defg-hij",
  "metadata": {
    "event_id": "abc123xyz",
    "calendar_id": "primary",
    "end_time": "2025-08-28T09:30:00Z",
    "location": "Conference Room A",
    "meeting_link": "https://meet.google.com/abc-defg-hij",
    "attendees": "5",
    "organizer": "manager@example.com",
    "event_type": "standup"
  }
}
```

## 🎨 Advanced Features

### 1. Multi-Calendar Support

```rust
async fn watch_multiple_calendars(&self, calendar_ids: Vec<String>) {
    for calendar_id in calendar_ids {
        let adapter = self.clone();
        tokio::spawn(async move {
            adapter.watch_calendar(&calendar_id).await
        });
    }
}
```

### 2. Smart Event Detection

```rust
// Detect and handle recurring events
fn is_recurring(&self, event: &Event) -> bool {
    event.recurring_event_id.is_some()
}

// Skip declined events
fn should_track(&self, event: &Event) -> bool {
    if let Some(attendees) = &event.attendees {
        for attendee in attendees {
            if attendee.self_.unwrap_or(false) {
                return attendee.response_status != Some("declined".to_string());
            }
        }
    }
    true
}
```

### 3. Analytics Integration

```rust
// Track calendar productivity statistics
struct CalendarAnalytics {
    // Meetings
    total_meetings_today: u32,
    total_meeting_time: Duration,
    average_meeting_length: Duration,
    back_to_back_meetings: u32,
    most_frequent_attendees: Vec<String>,
    // Focus
    free_windows_today: u32,
    total_free_time: Duration,
    avg_free_window: Duration,
    // Ratios
    focus_vs_meeting_ratio: f64, // total_free_time / (total_free_time + total_meeting_time)
}

impl CalendarAdapter {
    async fn generate_daily_stats(&self) -> CalendarAnalytics {
        // Analyze meetings vs focus windows (working hours only)
        // Examples:
        //  - time spent in meetings vs focus time (ratio)
        //  - most frequent meeting attendees (top N emails)
        //  - average free window length and count
        //  - number of back-to-back meetings
        // Optionally: publish a summary race or write to a dashboard
    }
}
```

### 4. Webhook Server

```rust
// Receive real-time updates from Google Calendar
async fn start_webhook_server(&self, port: u16) {
    let app = Router::new()
        .route("/webhook/calendar", post(handle_calendar_webhook))
        .with_state(self.clone());
    
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_calendar_webhook(
    State(adapter): State<CalendarAdapter>,
    Json(notification): Json<CalendarNotification>,
) -> impl IntoResponse {
    // Process calendar change notification
    adapter.sync_events().await;
    StatusCode::OK
}
```

## 🔒 Security Considerations

1. **OAuth Tokens**: Store securely, use OS keyring when possible
2. **Webhook Validation**: Verify Google's push notifications
3. **Rate Limiting**: Respect Google Calendar API quotas
4. **Privacy**: Option to exclude private event details
5. **Encryption**: Encrypt stored credentials and tokens
6. **X-Goog-Channel-Token**: For push notifications, set and verify the `X-Goog-Channel-Token` header on incoming webhook requests to ensure messages are intended for your channel.

## 🐛 Troubleshooting

### Common Issues

1. **Authentication Errors**:
   - Delete `calendar_tokens.json` and re-authenticate
   - Check OAuth consent screen configuration

2. **Missing Events**:
   - Verify calendar ID is correct
   - Check timezone settings
   - Ensure API has calendar.readonly scope

3. **Webhook Not Working**:
   - Ensure public HTTPS URL
   - Check SSL certificate validity
   - Verify domain ownership in Google Console

## 📈 Monitoring

```rust
// Health check endpoint
async fn health_check(&self) -> HealthStatus {
    HealthStatus {
        google_api: self.test_api_connection().await,
        raceboard: self.test_raceboard_connection().await,
        last_sync: self.last_sync_time,
        events_tracked: self.events_count,
    }
}
```

## 🔗 Related Documentation

- [Google Calendar API](https://developers.google.com/calendar/api/v3/reference)
- [OAuth 2.0 for Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Raceboard API Documentation](../README.md)
- [Claude Adapter](./CLAUDE_ADAPTER.md)
- [Codex Adapter](./CODEX_ADAPTER.md)
```rust
impl CalendarAdapter {
    async fn sync_free_windows(&self) -> Result<()> {
        // 1) Resolve working window for today in local timezone
        let now = Utc::now();
        let (work_start, work_end, tz) = self.resolve_working_window(now)?;
        if now < work_start || now >= work_end { return Ok(()); }

        // 2) Fetch events within working window
        let events = self.hub.events()
            .list("primary")
            .time_min(&work_start.to_rfc3339())
            .time_max(&work_end.to_rfc3339())
            .single_events(true)
            .order_by("startTime")
            .doit()
            .await?;
        // 3) Build busy intervals (exclude focusTime, include real meetings)
        let busy = self.collect_busy_intervals(events.1.items.unwrap_or_default());

        // 4) If we are free now, compute window [start,end)
        if !self.is_busy(now, &busy) {
            let window_start = self.last_busy_end_before(now, &busy).unwrap_or(work_start);
            let window_end = self.next_busy_start_after(now, &busy).unwrap_or(work_end);
            let next_title = self.next_meeting_title_after(now, &busy);
            self.ensure_free_window_race(window_start, window_end, next_title, &tz).await?;
        } else {
            self.finish_free_window_if_any().await?;
        }
        Ok(())
    }

    // Helper stubs
    fn resolve_working_window(&self, _now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>, String)> { unimplemented!() }
    fn collect_busy_intervals(&self, _events: Vec<Event>) -> Vec<(DateTime<Utc>, DateTime<Utc>, String)> { vec![] }
    fn is_busy(&self, _t: DateTime<Utc>, _busy: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> bool { false }
    fn last_busy_end_before(&self, _t: DateTime<Utc>, _busy: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> Option<DateTime<Utc>> { None }
    fn next_busy_start_after(&self, _t: DateTime<Utc>, _busy: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> Option<DateTime<Utc>> { None }
    fn next_meeting_title_after(&self, _t: DateTime<Utc>, _busy: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> Option<String> { None }
    async fn ensure_free_window_race(&self, _start: DateTime<Utc>, _end: DateTime<Utc>, _next: Option<String>, _tz: &str) -> Result<()> { Ok(()) }
    async fn finish_free_window_if_any(&self) -> Result<()> { Ok(()) }
}
```
