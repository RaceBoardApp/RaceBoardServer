use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeZone, Utc};
use clap::Parser;
use google_calendar3 as gcal;
use hyper::client::HttpConnector;
use hyper_rustls::HttpsConnector;
use RaceboardServer::adapter_common::{
    load_config_file, RaceboardClient, Race, RaceState, RaceUpdate, ServerConfig,
    AdapterType, AdapterHealthMonitor
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use tokio::time::interval;
use yup_oauth2 as oauth2;

// Race, RaceState, and RaceUpdate are now imported from adapter_common

#[derive(Parser, Debug)]
#[command(author, version, about = "Raceboard Google Calendar free-time adapter")]
struct Args {
    /// Raceboard server URL
    #[arg(short = 's', long, default_value = "http://localhost:7777")]
    server: String,

    /// Path to config TOML (optional)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Poll interval seconds
    #[arg(long, default_value_t = 30)]
    interval: u64,

    /// Enable debug logs
    #[arg(long, default_value_t = false)]
    debug: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkingHoursCfg {
    enabled: Option<bool>,
    start: Option<String>, // HH:MM local
    end: Option<String>,   // HH:MM local
}

#[derive(Debug, Clone, Deserialize)]
struct FocusTimeCfg {
    enabled: Option<bool>,
    title_patterns: Option<Vec<String>>, // fallback when eventType not exposed
}

#[derive(Debug, Clone, Deserialize)]
struct SyncCfg {
    interval_seconds: Option<u64>,
    lookahead_hours: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct FiltersCfg {
    calendars: Option<Vec<String>>,
    ignore_all_day_events: Option<bool>,
    min_duration_minutes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppConfig {
    raceboard: Option<ServerConfig>,
    sync: Option<SyncCfg>,
    filters: Option<FiltersCfg>,
    working_hours: Option<WorkingHoursCfg>,
    focus_time: Option<FocusTimeCfg>,
    google: Option<GoogleCfg>,
    ics: Option<IcsCfg>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleCfg {
    credentials_path: Option<String>,
    token_cache: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct IcsCfg {
    url: Option<String>,
}

#[derive(Debug, Clone)]
struct EffectiveCfg {
    server_config: ServerConfig,
    poll_interval: StdDuration,
    work_start: NaiveTime,
    work_end: NaiveTime,
    focus_patterns: Vec<String>,
    google_credentials: Option<String>,
    google_token_cache: Option<String>,
    ics_url: Option<String>,
    ignore_all_day: bool,
    min_duration_minutes: Option<i64>,
}

impl EffectiveCfg {
    fn from_args_config(args: &Args, file: Option<AppConfig>) -> Self {
        let server_config = file
            .as_ref()
            .and_then(|c| c.raceboard.clone())
            .unwrap_or_else(|| ServerConfig {
                url: args.server.clone(),
                ..Default::default()
            });

        let poll_secs = file
            .as_ref()
            .and_then(|c| c.sync.as_ref())
            .and_then(|s| s.interval_seconds)
            .unwrap_or(args.interval)
            .max(5);

        // Defaults 10:00–18:00 local
        let (ws, we) = (
            file.as_ref()
                .and_then(|c| c.working_hours.as_ref())
                .and_then(|w| w.start.clone())
                .unwrap_or_else(|| "10:00".to_string()),
            file.as_ref()
                .and_then(|c| c.working_hours.as_ref())
                .and_then(|w| w.end.clone())
                .unwrap_or_else(|| "18:00".to_string()),
        );
        let work_start = NaiveTime::parse_from_str(&ws, "%H:%M")
            .unwrap_or(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        let work_end = NaiveTime::parse_from_str(&we, "%H:%M")
            .unwrap_or(NaiveTime::from_hms_opt(18, 0, 0).unwrap());

        let focus_patterns = file
            .as_ref()
            .and_then(|c| c.focus_time.as_ref())
            .and_then(|f| f.title_patterns.clone())
            .unwrap_or_else(|| vec!["Focus".to_string(), "Deep work".to_string()]);

        Self {
            server_config,
            poll_interval: StdDuration::from_secs(poll_secs),
            work_start,
            work_end,
            focus_patterns,
            google_credentials: file
                .as_ref()
                .and_then(|c| c.google.as_ref())
                .and_then(|g| g.credentials_path.clone()),
            google_token_cache: file
                .as_ref()
                .and_then(|c| c.google.as_ref())
                .and_then(|g| g.token_cache.clone()),
            ics_url: file
                .as_ref()
                .and_then(|c| c.ics.as_ref())
                .and_then(|i| i.url.clone()),
            ignore_all_day: file
                .as_ref()
                .and_then(|c| c.filters.as_ref())
                .and_then(|f| f.ignore_all_day_events)
                .unwrap_or(true),
            min_duration_minutes: file
                .as_ref()
                .and_then(|c| c.filters.as_ref())
                .and_then(|f| f.min_duration_minutes),
        }
    }
}

#[derive(Debug, Clone)]
struct FreeRaceState {
    race_id: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct EventItem {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    title: String,
    event_type: Option<String>, // e.g., "focusTime"
}

#[async_trait::async_trait]
trait CalendarProvider: Send + Sync {
    async fn fetch_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<EventItem>>;
}

struct PlaceholderProvider;

#[async_trait::async_trait]
impl CalendarProvider for PlaceholderProvider {
    async fn fetch_events(
        &self,
        _start: DateTime<Utc>,
        _end: DateTime<Utc>,
    ) -> Result<Vec<EventItem>> {
        Ok(vec![])
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.debug {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();

    let cfg = if let Some(path) = args.config.as_ref() {
        let f: AppConfig = load_config_file(Some(path.clone())).context("loading config file")?;
        EffectiveCfg::from_args_config(&args, Some(f))
    } else {
        EffectiveCfg::from_args_config(&args, None)
    };

    let raceboard_client = RaceboardClient::new(cfg.server_config.clone())
        .context("Failed to create raceboard client")?;
    
    // Create and register health monitor
    let instance_id = format!("calendar-{}", std::process::id());
    let mut health_monitor = AdapterHealthMonitor::new(
        raceboard_client.clone(),
        AdapterType::Calendar,
        instance_id.clone(),
        60, // Report health every 60 seconds
    ).await.context("Failed to create health monitor")?;
    
    log::info!("Registering calendar adapter with Raceboard server...");
    health_monitor.register().await
        .context("Failed to register adapter with server")?;
    log::info!("Adapter registered successfully as: adapter:calendar:{}", instance_id);
    
    // Start automatic health reporting
    let health_report_handle = tokio::spawn(async move {
        health_monitor.clone().start_health_reporting().await;
    });
    
    let mut state: Option<FreeRaceState> = None;
    let mut tick = interval(cfg.poll_interval);
    loop {
        tick.tick().await;
        if let Err(e) = sync_once(&cfg, &raceboard_client, &mut state).await {
            eprintln!("[calendar] sync error: {e}");
        }
    }
}

async fn sync_once(
    cfg: &EffectiveCfg,
    client: &RaceboardClient,
    state: &mut Option<FreeRaceState>,
) -> Result<()> {
    // Resolve working window today in local tz
    let now_local = Local::now();
    let today = now_local.date_naive();
    let start_local = today.and_time(cfg.work_start);
    let end_local = today.and_time(cfg.work_end);
    let work_start: DateTime<Utc> = {
        let lr = Local.from_local_datetime(&start_local);
        lr.single()
            .or_else(|| lr.earliest())
            .expect("valid local start time")
            .with_timezone(&Utc)
    };
    let work_end: DateTime<Utc> = {
        let lr = Local.from_local_datetime(&end_local);
        lr.single()
            .or_else(|| lr.latest())
            .expect("valid local end time")
            .with_timezone(&Utc)
    };

    if Utc::now() < work_start || Utc::now() >= work_end {
        // Outside working hours: finish any active race
        if let Some(active) = state.take() {
            finish_race(client, &active.race_id)
                .await
                .ok();
        }
        return Ok(());
    }

    // Choose provider: ICS URL if provided, then Google; otherwise placeholder
    let events = if let Some(url) = cfg.ics_url.clone() {
        let provider = IcsProvider::new(&url);
        provider.fetch_events(work_start, work_end).await?
    } else if let Some(creds) = cfg.google_credentials.clone() {
        let token = cfg
            .google_token_cache
            .clone()
            .unwrap_or_else(|| "calendar_tokens.json".to_string());
        let provider = GoogleProvider::new(&creds, &token).await?;
        provider.fetch_events(work_start, work_end).await?
    } else {
        let provider = PlaceholderProvider;
        provider.fetch_events(work_start, work_end).await?
    };
    let busy = collect_busy_intervals(
        &events,
        &cfg.focus_patterns,
        cfg.ignore_all_day,
        cfg.min_duration_minutes,
    );

    let now = Utc::now();
    let free_now = !is_busy(now, &busy);
    if free_now {
        let window_start = last_busy_end_before(now, &busy).unwrap_or(work_start);
        let window_end = next_busy_start_after(now, &busy).unwrap_or(work_end);
        let next_title = next_meeting_title_after(now, &busy);
        ensure_free_window_race(
            client,
            state,
            window_start,
            window_end,
            next_title,
        )
        .await?;
    } else if let Some(active) = state.take() {
        // If we now have a meeting, finish the active free race
        finish_race(client, &active.race_id).await?;
    }

    // If a race is active, update progress/ETA
    if let Some(active) = state.as_mut() {
        let now = Utc::now();
        if now >= active.window_end {
            finish_race(client, &active.race_id).await?;
            *state = None;
        } else {
            let dur = (active.window_end - active.window_start)
                .num_seconds()
                .max(1);
            let elapsed = (now - active.window_start).num_seconds().max(0);
            let progress = ((elapsed as f64 / dur as f64) * 100.0) as i32;
            let eta = (active.window_end - now).num_seconds();
            patch_progress(
                client,
                &active.race_id,
                progress.clamp(0, 99),
                eta,
            )
            .await?;
        }
    }

    Ok(())
}

async fn fetch_events_placeholder(
    _start: DateTime<Utc>,
    _end: DateTime<Utc>,
) -> Result<Vec<EventItem>> {
    // TODO: integrate Google Calendar API
    Ok(vec![])
}

fn collect_busy_intervals(
    events: &[EventItem],
    focus_patterns: &[String],
    ignore_all_day: bool,
    min_duration_minutes: Option<i64>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, String)> {
    let mut busy = Vec::new();
    'outer: for e in events {
        // Exclude focus time
        if let Some(t) = &e.event_type {
            if t.eq_ignore_ascii_case("focusTime") {
                continue 'outer;
            }
            if ignore_all_day && t.eq_ignore_ascii_case("all_day") {
                continue 'outer;
            }
        }
        for pat in focus_patterns {
            if !e.title.is_empty() && e.title.to_lowercase().contains(&pat.to_lowercase()) {
                continue 'outer;
            }
        }
        if let Some(minm) = min_duration_minutes {
            let mins = (e.end - e.start).num_minutes();
            if mins < minm {
                continue 'outer;
            }
        }
        busy.push((e.start, e.end, e.title.clone()));
    }
    busy.sort_by_key(|(s, _, _)| *s);
    busy
}

fn is_busy(t: DateTime<Utc>, busy: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> bool {
    busy.iter().any(|(s, e, _)| *s <= t && t < *e)
}

fn last_busy_end_before(
    t: DateTime<Utc>,
    busy: &[(DateTime<Utc>, DateTime<Utc>, String)],
) -> Option<DateTime<Utc>> {
    busy.iter()
        .filter(|(s, e, _)| *e <= t || (*s <= t && *e <= t))
        .map(|(_, e, _)| *e)
        .max()
}

fn next_busy_start_after(
    t: DateTime<Utc>,
    busy: &[(DateTime<Utc>, DateTime<Utc>, String)],
) -> Option<DateTime<Utc>> {
    busy.iter()
        .filter(|(s, _, _)| *s > t)
        .map(|(s, _, _)| *s)
        .min()
}

fn next_meeting_title_after(
    t: DateTime<Utc>,
    busy: &[(DateTime<Utc>, DateTime<Utc>, String)],
) -> Option<String> {
    busy.iter()
        .filter(|(s, _, _)| *s > t)
        .map(|(_, _, title)| title.clone())
        .min()
}

async fn ensure_free_window_race(
    client: &RaceboardClient,
    state: &mut Option<FreeRaceState>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    next_title: Option<String>,
) -> Result<()> {
    let now = Utc::now();
    // If we already have active race for same window, nothing to do
    if let Some(active) = state.as_ref() {
        if active.window_start == window_start && active.window_end == window_end {
            return Ok(());
        }
    }

    // Finish previous if exists
    if let Some(prev) = state.take() {
        finish_race(client, &prev.race_id).await.ok();
    }

    let eta_sec = (window_end - now).num_seconds().max(0);
    let dur = (window_end - window_start).num_seconds().max(1);
    let elapsed = (now - window_start).num_seconds().max(0);
    let progress = ((elapsed as f64 / dur as f64) * 100.0) as i32;
    let id = format!(
        "gcal:free:{}-{}",
        window_start.format("%Y%m%dT%H%M"),
        window_end.format("%Y%m%dT%H%M")
    );
    let title = next_title
        .map(|t| format!("Until next meeting: {}", t))
        .unwrap_or_else(|| "Free time".to_string());
    let mut metadata = HashMap::new();
    metadata.insert(
        "window_start".to_string(),
        window_start.to_rfc3339(),
    );
    metadata.insert(
        "window_end".to_string(),
        window_end.to_rfc3339(),
    );

    let race = Race {
        id: id.clone(),
        source: "google-calendar".to_string(),
        title,
        state: RaceState::Running,
        started_at: window_start,
        eta_sec: Some(eta_sec),
        progress: Some(progress.clamp(0, 99)),
        deeplink: None,
        metadata: Some(metadata),
    };
    let _created_race = client.create_race(&race).await.context("create race")?;
    *state = Some(FreeRaceState {
        race_id: id,
        window_start,
        window_end,
    });
    Ok(())
}

async fn patch_progress(
    client: &RaceboardClient,
    race_id: &str,
    progress: i32,
    eta_sec: i64,
) -> Result<()> {
    let update = RaceUpdate {
        state: None,
        progress: Some(progress),
        eta_sec: Some(eta_sec),
        metadata: None,
        deeplink: None,
    };
    client.update_race(race_id, &update).await.context("update race progress")?;
    Ok(())
}

async fn finish_race(client: &RaceboardClient, race_id: &str) -> Result<()> {
    let update = RaceUpdate {
        state: Some(RaceState::Passed),
        progress: Some(100),
        eta_sec: Some(0),
        metadata: None,
        deeplink: None,
    };
    client.update_race(race_id, &update).await.context("finish race")?;
    Ok(())
}

// ========== Google Calendar Provider (OAuth + Events) ==========

struct GoogleProvider {
    hub: gcal::CalendarHub<HttpsConnector<HttpConnector>>, // authorized client
}

impl GoogleProvider {
    async fn new(credentials_path: &str, token_cache: &str) -> Result<Self> {
        let secret = oauth2::read_application_secret(credentials_path).await?;
        let auth: oauth2::authenticator::Authenticator<_> =
            oauth2::InstalledFlowAuthenticator::builder(
                secret,
                oauth2::InstalledFlowReturnMethod::HTTPRedirect,
            )
            .persist_tokens_to_disk(token_cache)
            .build()
            .await?;

        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .build();
        let client = hyper::Client::builder().build::<_, hyper::Body>(https);
        let hub = gcal::CalendarHub::new(client, auth);
        Ok(Self { hub })
    }
}

#[async_trait::async_trait]
impl CalendarProvider for GoogleProvider {
    async fn fetch_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<EventItem>> {
        let items = self
            .hub
            .events()
            .list("primary")
            .time_min(start)
            .time_max(end)
            .single_events(true)
            .order_by("startTime")
            .doit()
            .await
            .context("google events list")?
            .1
            .items
            .unwrap_or_default();

        let mut out = Vec::new();
        for ev in items {
            // Skip all-day events (date only)
            let Some(st) = ev.start else { continue };
            let Some(en) = ev.end else { continue };
            let start_dt = match parse_event_time(&st) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let end_dt = match parse_event_time(&en) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let title = ev.summary.unwrap_or_else(|| "(untitled)".to_string());
            let et = ev.event_type; // Some("focusTime") on newer APIs
            out.push(EventItem {
                start: start_dt,
                end: end_dt,
                title,
                event_type: et,
            });
        }
        Ok(out)
    }
}

fn parse_event_time(t: &gcal::api::EventDateTime) -> Result<DateTime<Utc>> {
    if let Some(dt) = &t.date_time {
        Ok(dt.clone())
    } else if let Some(_d) = &t.date {
        // All-day: caller should skip
        anyhow::bail!("all-day event")
    } else {
        anyhow::bail!("missing event time")
    }
}

// ========== ICS Provider (no Google Cloud needed) ==========

struct IcsProvider {
    url: String,
}

impl IcsProvider {
    fn new(url: &str) -> Self {
        Self { url: url.to_string() }
    }

    fn parse_ics_datetime(value: &str, tzid: Option<&str>) -> Option<DateTime<Utc>> {
        // All-day event format YYYYMMDD
        if value.len() == 8 && !value.contains('T') {
            return None;
        }
        // UTC with trailing Z
        if value.ends_with('Z') {
            if let Ok(dt) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ") {
                return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
            }
        }
        // Local or with TZID
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
            if let Some(tz) = tzid.and_then(|s| s.parse::<chrono_tz::Tz>().ok()) {
                return tz.from_local_datetime(&dt).single().map(|d| d.with_timezone(&Utc));
            } else {
                let lr = Local.from_local_datetime(&dt);
                return lr.single().or_else(|| lr.earliest()).map(|d| d.with_timezone(&Utc));
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl CalendarProvider for IcsProvider {
    async fn fetch_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<EventItem>> {
        let text = reqwest::get(&self.url).await?.text().await?;
        let mut out = Vec::new();
        let reader = std::io::Cursor::new(text);
        let parser = ical::IcalParser::new(reader);
        for item in parser {
            let Ok(cal) = item else { continue };
            for comp in cal.events {
                let mut summary: Option<String> = None;
                let mut dtstart: Option<(String, Option<String>)> = None;
                let mut dtend: Option<(String, Option<String>)> = None;
                for prop in comp.properties {
                    let name = prop.name.to_uppercase();
                    match name.as_str() {
                        "SUMMARY" => summary = prop.value,
                        "DTSTART" => {
                            let tzid = prop.params.as_ref().and_then(|pairs| {
                                pairs
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("TZID"))
                                    .and_then(|(_, v)| v.first().cloned())
                            });
                            dtstart = Some((prop.value.unwrap_or_default(), tzid));
                        }
                        "DTEND" => {
                            let tzid = prop.params.as_ref().and_then(|pairs| {
                                pairs
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("TZID"))
                                    .and_then(|(_, v)| v.first().cloned())
                            });
                            dtend = Some((prop.value.unwrap_or_default(), tzid));
                        }
                        _ => {}
                    }
                }
                let Some((s_raw, s_tz)) = dtstart else { continue };
                let Some((e_raw, e_tz)) = dtend else { continue };
                let Some(st) = IcsProvider::parse_ics_datetime(&s_raw, s_tz.as_deref()) else { continue };
                let Some(en) = IcsProvider::parse_ics_datetime(&e_raw, e_tz.as_deref()) else { continue };
                if en <= st { continue; }
                if en <= start || st >= end { continue; }
                out.push(EventItem { start: st, end: en, title: summary.clone().unwrap_or_else(|| "(untitled)".to_string()), event_type: None });
            }
        }
        Ok(out)
    }
}
