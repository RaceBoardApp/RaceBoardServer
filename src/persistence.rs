use crate::cluster::RaceCluster;
use crate::models::Race;
use crate::prediction::SourceStats;
use anyhow::Result;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;

#[async_trait]
pub trait RaceStore: Send + Sync {
    async fn get_all_races(&self) -> Result<Vec<Race>>;
    async fn store_race(&self, race: &Race) -> Result<()>;
    async fn delete_race(&self, race_id: &str) -> Result<()>;
}

#[derive(Debug)]
pub struct PersistenceLayer {
    db: sled::Db,
    races_tree: sled::Tree,
    races_by_time: sled::Tree,
    clusters_tree: sled::Tree,
    source_stats_tree: sled::Tree,
    meta_tree: sled::Tree,
}

impl PersistenceLayer {
    pub fn new_in_memory() -> Result<Self> {
        // Create an in-memory sled database
        let db = sled::Config::new().temporary(true).open()?;
        let races_tree = db.open_tree("races")?;
        let races_by_time = db.open_tree("races_by_time")?;
        let clusters_tree = db.open_tree("clusters")?;
        let source_stats_tree = db.open_tree("source_stats")?;
        let meta_tree = db.open_tree("meta")?;
        Ok(Self {
            db,
            races_tree,
            races_by_time,
            clusters_tree,
            source_stats_tree,
            meta_tree,
        })
    }

    pub fn new(db_path: Option<PathBuf>) -> Result<Self> {
        let path = db_path.unwrap_or_else(|| {
            let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            path.push(".raceboard");
            path.push("eta_history.db");
            path
        });

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Try to open the database - fail if already locked
        match sled::open(&path) {
            Ok(db) => {
                let races_tree = db.open_tree("races")?;
                let races_by_time = db.open_tree("races_by_time")?;
                let clusters_tree = db.open_tree("clusters")?;
                let source_stats_tree = db.open_tree("source_stats")?;
                let meta_tree = db.open_tree("meta")?;
                log::info!("Successfully opened sled database at {:?}", path);
                let layer = Self {
                    db,
                    races_tree,
                    races_by_time,
                    clusters_tree,
                    source_stats_tree,
                    meta_tree,
                };
                layer.ensure_schema_version(2)?;
                Ok(layer)
            }
            Err(e) => {
                // If database is locked, it means another instance is running
                if e.to_string().contains("could not acquire lock")
                    || e.to_string().contains("Resource temporarily unavailable")
                {
                    eprintln!("ERROR: Database is locked at {:?}", path);
                    eprintln!("Another instance of the server is likely running.");
                    eprintln!("Please stop it first with: pkill -f raceboard-server");
                    std::process::exit(1);
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub fn persist_cluster(&self, cluster: &RaceCluster) -> Result<()> {
        let key = cluster.cluster_id.as_bytes();
        let value = self.serialize_enveloped(cluster, "RaceCluster@2")?;
        // Write to dedicated clusters tree
        self.clusters_tree.insert(key, value)?;
        self.clusters_tree.flush()?;
        Ok(())
    }

    pub fn load_clusters(&self) -> Result<HashMap<String, RaceCluster>> {
        let mut clusters = HashMap::new();
        // Preferred: load from dedicated clusters tree
        for item in self.clusters_tree.iter() {
            let (key, value) = item?;
            let cluster_id = String::from_utf8_lossy(&key).to_string();

            match self.deserialize_enveloped::<RaceCluster>(&value) {
                Ok(cluster) => {
                    if validate_cluster_data(&cluster).is_ok() {
                        clusters.insert(cluster_id, cluster);
                    } else {
                        eprintln!("Invalid cluster data for {}, skipping", cluster_id);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to deserialize cluster {}: {}", cluster_id, e);
                }
            }
        }
        // Backward compatibility: also scan default tree for legacy clusters
        for item in self.db.iter() {
            let (key, value) = item?;
            let cluster_id = String::from_utf8_lossy(&key).to_string();
            if cluster_id.starts_with("source:") {
                continue;
            }
            if cluster_id == "races" || cluster_id == "races_by_time" {
                continue;
            }
            if clusters.contains_key(&cluster_id) {
                continue;
            }
            if let Ok(cluster) = bincode::deserialize::<RaceCluster>(&value) {
                if validate_cluster_data(&cluster).is_ok() {
                    clusters.insert(cluster_id, cluster);
                }
            }
        }
        Ok(clusters)
    }

    /// Clear all clusters from persistence
    pub fn clear_clusters(&self) -> Result<()> {
        log::info!("Clearing all clusters from persistence");
        self.clusters_tree.clear()?;
        self.clusters_tree.flush()?;
        Ok(())
    }

    pub fn delete_cluster(&self, cluster_id: &str) -> Result<()> {
        self.db.remove(cluster_id.as_bytes())?;
        self.db.flush()?;
        Ok(())
    }

    pub fn persist_all_clusters(&self, clusters: &HashMap<String, RaceCluster>) -> Result<()> {
        for cluster in clusters.values() {
            self.persist_cluster(cluster)?;
        }
        Ok(())
    }

    pub fn cleanup_old_data(&self, ttl_days: u32) -> Result<usize> {
        use chrono::{Duration, Utc};

        let cutoff = Utc::now() - Duration::days(ttl_days as i64);
        let mut deleted_count = 0;

        for item in self.db.iter() {
            let (key, value) = item?;

            if let Ok(cluster) = bincode::deserialize::<RaceCluster>(&value) {
                if cluster.last_accessed < cutoff {
                    self.db.remove(key)?;
                    deleted_count += 1;
                }
            }
        }

        self.db.flush()?;
        Ok(deleted_count)
    }

    pub fn get_db_size(&self) -> Result<u64> {
        Ok(self.db.size_on_disk()?)
    }

    // Source stats persistence methods
    pub fn persist_source_stats(&self, source: &str, stats: &SourceStats) -> Result<()> {
        let key = source.as_bytes();
        let value = self.serialize_enveloped(stats, "SourceStats@2")?;
        self.source_stats_tree.insert(key, value)?;
        self.source_stats_tree.flush()?;
        Ok(())
    }

    pub fn load_source_stats(&self) -> Result<HashMap<String, SourceStats>> {
        let mut stats = HashMap::new();
        // Preferred: load from dedicated tree
        for item in self.source_stats_tree.iter() {
            let (key, value) = item?;
            let source = String::from_utf8_lossy(&key).to_string();
            match self.deserialize_enveloped::<SourceStats>(&value) {
                Ok(source_stats) => {
                    stats.insert(source, source_stats);
                }
                Err(e) => {
                    eprintln!("Failed to deserialize source stats for {}: {}", source, e);
                }
            }
        }
        // Backward compatibility: load any legacy entries with prefix source:
        for item in self.db.iter() {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with("source:") {
                continue;
            }
            let source = key_str.strip_prefix("source:").unwrap().to_string();
            if stats.contains_key(&source) {
                continue;
            }
            if let Ok(source_stats) = bincode::deserialize::<SourceStats>(&value) {
                stats.insert(source, source_stats);
            }
        }

        Ok(stats)
    }

    pub fn delete_source_stats(&self, source: &str) -> Result<()> {
        let key = source.as_bytes();
        self.source_stats_tree.remove(key)?;
        self.source_stats_tree.flush()?;
        Ok(())
    }
}

#[async_trait]
impl RaceStore for PersistenceLayer {
    async fn get_all_races(&self) -> Result<Vec<Race>> {
        log::warn!(
            "PERSISTENCE: get_all_races called, tree has {} items",
            self.races_tree.len()
        );
        let mut races = Vec::new();

        for item in self.races_tree.iter() {
            let (key, value) = item?;
            let id = String::from_utf8_lossy(&key);
            log::warn!("PERSISTENCE: Found race key: {}", id);
            match self.deserialize_enveloped::<Race>(&value) {
                Ok(race) => races.push(race),
                Err(e) => {
                    // Try legacy decode
                    match bincode::deserialize::<Race>(&value) {
                        Ok(race) => races.push(race),
                        Err(e2) => {
                            log::error!(
                                "PERSISTENCE: Failed to deserialize race {}: {} / legacy: {}",
                                id,
                                e,
                                e2
                            );
                            log::error!("PERSISTENCE: Data length: {} bytes", value.len());
                        }
                    }
                }
            }
        }

        log::warn!("PERSISTENCE: Returning {} races", races.len());
        Ok(races)
    }

    async fn store_race(&self, race: &Race) -> Result<()> {
        log::warn!("PERSISTENCE: Storing race {}", race.id);
        let key = race.id.as_bytes();
        let value = self.serialize_enveloped(race, "Race@2")?;
        // Maintain time index (remove old if started_at changed)
        if let Ok(Some(old)) = self.races_tree.get(key) {
            // Try envelope first, then legacy
            if let Ok(old_race) = self.deserialize_enveloped::<Race>(&old) {
                let old_idx = Self::encode_time_index(&old_race.started_at, &old_race.id);
                let _ = self.races_by_time.remove(old_idx);
            } else if let Ok(old_race) = bincode::deserialize::<Race>(&old) {
                let old_idx = Self::encode_time_index(&old_race.started_at, &old_race.id);
                let _ = self.races_by_time.remove(old_idx);
            }
        }
        log::warn!(
            "PERSISTENCE: Serialized race {} to {} bytes",
            race.id,
            value.len()
        );
        self.races_tree.insert(key, value.clone())?;
        log::warn!("PERSISTENCE: Inserted race {} into tree", race.id);

        // Debug: immediately try to read it back
        if let Ok(Some(stored)) = self.races_tree.get(key) {
            log::warn!(
                "PERSISTENCE: Read back {} bytes for race {}",
                stored.len(),
                race.id
            );
            if stored.len() != value.len() {
                log::error!(
                    "PERSISTENCE: SIZE MISMATCH! Stored {} vs original {}",
                    stored.len(),
                    value.len()
                );
            }
            // Try to deserialize immediately
            match self.deserialize_enveloped::<Race>(&stored) {
                Ok(_) => log::warn!(
                    "PERSISTENCE: Successfully deserialized race {} immediately after store",
                    race.id
                ),
                Err(e) => log::error!(
                    "PERSISTENCE: Failed to deserialize race {} immediately after store: {}",
                    race.id,
                    e
                ),
            }
        }
        // Insert new index key
        let idx_key = Self::encode_time_index(&race.started_at, &race.id);
        self.races_by_time.insert(idx_key, &[])?;
        self.races_tree.flush()?;
        self.races_by_time.flush()?;
        log::warn!("PERSISTENCE: Flushed race {} to disk", race.id);

        // Verify it was stored
        if let Ok(Some(_)) = self.races_tree.get(key) {
            log::warn!("PERSISTENCE: Verified race {} exists in tree", race.id);
        } else {
            log::error!("PERSISTENCE: Race {} NOT FOUND after store!", race.id);
        }

        Ok(())
    }

    async fn delete_race(&self, race_id: &str) -> Result<()> {
        // Remove from index first (best effort)
        if let Ok(Some(val)) = self.races_tree.get(race_id.as_bytes()) {
            if let Ok(r) = self.deserialize_enveloped::<Race>(&val) {
                let idx_key = Self::encode_time_index(&r.started_at, race_id);
                let _ = self.races_by_time.remove(idx_key);
            } else if let Ok(r) = bincode::deserialize::<Race>(&val) {
                let idx_key = Self::encode_time_index(&r.started_at, race_id);
                let _ = self.races_by_time.remove(idx_key);
            }
        }
        self.races_tree.remove(race_id.as_bytes())?;
        self.races_tree.flush()?;
        self.races_by_time.flush()?;
        Ok(())
    }
}

fn validate_cluster_data(cluster: &RaceCluster) -> Result<()> {
    if cluster.cluster_id.is_empty() {
        return Err(anyhow::anyhow!("Cluster ID cannot be empty"));
    }

    if cluster.source.is_empty() {
        return Err(anyhow::anyhow!("Cluster source cannot be empty"));
    }

    if cluster.stats.mean < 0.0 {
        return Err(anyhow::anyhow!("Mean cannot be negative"));
    }

    if cluster.stats.median < 0.0 {
        return Err(anyhow::anyhow!("Median cannot be negative"));
    }

    Ok(())
}

// === New: time-ordered scanning API for historical races ===

#[derive(Debug, Clone)]
pub struct RaceScanFilter {
    pub source: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub include_events: bool,
}

#[derive(Debug, Clone)]
pub struct RaceBatch {
    pub items: Vec<Race>,
    pub next_cursor: Option<String>,
}

impl PersistenceLayer {
    fn encode_time_index(ts: &DateTime<Utc>, race_id: &str) -> Vec<u8> {
        // Compose: 8 bytes seconds since epoch (big-endian, saturating at 0) + 4 bytes nanos + race_id bytes
        let secs_i64 = ts.timestamp();
        let u64_secs = if secs_i64 >= 0 { secs_i64 as u64 } else { 0u64 };
        let nanos = ts.timestamp_subsec_nanos();
        let mut key = Vec::with_capacity(8 + 4 + race_id.len());
        key.extend_from_slice(&u64_secs.to_be_bytes());
        key.extend_from_slice(&(nanos as u32).to_be_bytes());
        key.extend_from_slice(race_id.as_bytes());
        key
    }

    fn decode_cursor(cursor: &str) -> Option<(DateTime<Utc>, String)> {
        let bytes = general_purpose::STANDARD.decode(cursor).ok()?;
        let s = String::from_utf8(bytes).ok()?;
        let v: serde_json::Value = serde_json::from_str(&s).ok()?;
        let sec = v.get("sec")?.as_i64()?;
        let nanos = v.get("nanos")?.as_u64().unwrap_or(0) as u32;
        let id = v.get("id")?.as_str()?.to_string();
        let dt = DateTime::<Utc>::from_timestamp(sec, nanos)?;
        Some((dt, id))
    }

    fn encode_cursor(ts: &DateTime<Utc>, id: &str) -> String {
        let obj = serde_json::json!({
            "sec": ts.timestamp(),
            "nanos": ts.timestamp_subsec_nanos(),
            "id": id,
        });
        let s = obj.to_string();
        general_purpose::STANDARD.encode(s.as_bytes())
    }

    pub async fn scan_races(
        &self,
        filter: RaceScanFilter,
        batch_size: usize,
        cursor: Option<String>,
    ) -> Result<RaceBatch> {
        log::warn!(
            "SCAN: Starting scan with filter: source={:?}, from={:?}, to={:?}, batch_size={}",
            filter.source,
            filter.from,
            filter.to,
            batch_size
        );
        let start_key = if let Some(c) = cursor.as_ref().and_then(|c| Self::decode_cursor(c)) {
            let (ts, id) = c;
            // Start strictly after the cursor key
            let mut k = Self::encode_time_index(&ts, &id);
            k.push(0x00);
            k
        } else if let Some(from) = filter.from {
            // Lowest possible id after the from timestamp
            Self::encode_time_index(&from, "")
        } else {
            vec![] // start from beginning
        };

        let end_bound = if let Some(to) = filter.to {
            let mut k = Self::encode_time_index(&to, "~"); // tilde sorts after typical chars
            k.push(0xFF);
            Some(k)
        } else {
            None
        };

        log::warn!("SCAN: Index has {} entries", self.races_by_time.len());
        let range = match end_bound {
            Some(end) => self.races_by_time.range(start_key..=end),
            None => self.races_by_time.range(start_key..),
        };
        log::warn!("SCAN: Created range iterator");

        let mut items = Vec::with_capacity(batch_size);
        let mut last_ts: Option<DateTime<Utc>> = None;
        let mut last_id: Option<String> = None;

        let mut count = 0;
        for item in range {
            let (k, _) = item?;
            count += 1;
            if k.len() < 12 {
                log::warn!("SCAN: Skipping short key of len {}", k.len());
                continue;
            }
            let secs_be = &k[0..8];
            let nanos_be = &k[8..12];
            let id_bytes = &k[12..];
            let u64_secs = u64::from_be_bytes(secs_be.try_into().unwrap());
            let secs = u64_secs as i64;
            let nanos = u32::from_be_bytes(nanos_be.try_into().unwrap());
            let ts = DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now);
            let id = String::from_utf8_lossy(id_bytes).to_string();

            // Fetch race record
            if let Some(val) = self.races_tree.get(id.as_bytes())? {
                log::debug!("SCAN: Found race {} in tree, attempting deserialize", id);
                // Envelope or legacy
                if let Ok(mut race) = self.deserialize_enveloped::<Race>(&val) {
                    if let Some(ref src) = filter.source {
                        if &race.source != src {
                            log::debug!("SCAN: Filtering out race {} with source '{}' (looking for '{}')", 
                                id, race.source, src);
                            continue;
                        }
                    }
                    if !filter.include_events {
                        race.events = None;
                    }
                    items.push(race);
                    last_ts = Some(ts);
                    last_id = Some(id);
                    if items.len() >= batch_size {
                        break;
                    }
                } else if let Ok(mut race) = bincode::deserialize::<Race>(&val) {
                    log::warn!("SCAN: Deserialized race {} via legacy format", id);
                    if let Some(ref src) = filter.source {
                        if &race.source != src {
                            log::debug!("SCAN: Filtering out legacy race {} with source '{}' (looking for '{}')", 
                                id, race.source, src);
                            continue;
                        }
                    }
                    if !filter.include_events {
                        race.events = None;
                    }
                    items.push(race);
                    last_ts = Some(ts);
                    last_id = Some(id);
                    if items.len() >= batch_size {
                        break;
                    }
                } else {
                    // Log detailed error
                    match self.deserialize_enveloped::<Race>(&val) {
                        Err(e1) => match bincode::deserialize::<Race>(&val) {
                            Err(e2) => {
                                log::error!("SCAN: Failed to deserialize race {} - envelope: {}, legacy: {}", id, e1, e2);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            } else {
                log::warn!("SCAN: Race {} NOT found in races_tree!", id);
            }
        }

        log::warn!(
            "SCAN: Iterated {} index entries, collected {} items",
            count,
            items.len()
        );

        let next_cursor = if items.len() >= batch_size {
            if let (Some(ts), Some(id)) = (last_ts.as_ref(), last_id.as_ref()) {
                Some(Self::encode_cursor(ts, id))
            } else {
                None
            }
        } else {
            None
        };

        Ok(RaceBatch { items, next_cursor })
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    pub fn write_audit_record(&self, kind: &str, payload: &serde_json::Value) -> Result<()> {
        let key = format!("audit:{}:{}", kind, chrono::Utc::now().timestamp_nanos());
        let value = serde_json::to_vec(payload)?;
        self.meta_tree.insert(key.as_bytes(), value)?;
        self.meta_tree.flush()?;
        Ok(())
    }

    pub fn get_schema_version(&self) -> Option<String> {
        self.meta_tree
            .get(b"schema_version")
            .ok()
            .flatten()
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    pub fn races_count(&self) -> usize {
        self.races_tree.len()
    }

    pub fn index_entries(&self) -> usize {
        self.races_by_time.len()
    }

    pub fn is_migration_complete(&self) -> bool {
        self.meta_tree
            .get(b"__migration_complete__")
            .unwrap_or(None)
            .is_some()
    }

    pub fn mark_migration_complete(&self) -> Result<()> {
        self.meta_tree.insert(b"__migration_complete__", b"1")?;
        self.meta_tree.flush()?;
        Ok(())
    }
}

// ===== Serialization helpers =====
// NOTE: We use direct JSON serialization instead of the Envelope pattern
// due to compatibility issues with bincode and PhantomData.
// This decision was made after encountering deserialization failures.

impl PersistenceLayer {
    fn serialize_enveloped<T: serde::Serialize>(
        &self,
        value: &T,
        _schema_tag: &str,
    ) -> Result<Vec<u8>> {
        // Use JSON serialization for all data
        serde_json::to_vec(value).map_err(|e| anyhow::anyhow!("serialize failed: {}", e))
    }

    fn deserialize_enveloped<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> Result<T> {
        // Try JSON first (current format)
        match serde_json::from_slice(data) {
            Ok(v) => Ok(v),
            Err(_) => {
                // Fallback to bincode for backward compatibility with old data
                bincode::deserialize(data).map_err(|e| anyhow::anyhow!("deserialize failed: {}", e))
            }
        }
    }

    fn ensure_schema_version(&self, version: u32) -> Result<()> {
        let key = b"schema_version";
        if let Some(val) = self.meta_tree.get(key)? {
            let existing = String::from_utf8_lossy(&val);
            log::info!("Schema version present: {}", existing);
            return Ok(());
        }
        self.meta_tree.insert(key, version.to_string().as_bytes())?;
        self.meta_tree.flush()?;
        Ok(())
    }

    // JSON Snapshot functionality for disaster recovery
    pub async fn create_json_snapshot(&self) -> Result<()> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use sha2::{Digest, Sha256};
        use std::io::Write;

        // Determine snapshot path
        let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push(".raceboard");
        std::fs::create_dir_all(&path)?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("races.snapshot.{}.json.gz", timestamp);
        path.push(&filename);

        // Collect all races
        let mut races = Vec::new();
        for item in self.races_tree.iter() {
            let (_, value) = item?;
            match self.deserialize_enveloped::<Race>(&value) {
                Ok(race) => races.push(race),
                Err(e) => {
                    // Try legacy format
                    if let Ok(race) = bincode::deserialize::<Race>(&value) {
                        races.push(race);
                    } else {
                        log::error!("Failed to deserialize race for snapshot: {}", e);
                    }
                }
            }
        }

        // Serialize to JSON
        let json_data = serde_json::to_vec_pretty(&races)?;

        // Calculate SHA-256 checksum
        let mut hasher = Sha256::new();
        hasher.update(&json_data);
        let checksum = format!("{:x}", hasher.finalize());

        // Compress and write
        let file = std::fs::File::create(&path)?;
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&json_data)?;
        encoder.finish()?;

        // Write checksum file
        let mut checksum_path = path.clone();
        checksum_path.set_extension("sha256");
        std::fs::write(&checksum_path, &checksum)?;

        // Record snapshot metadata
        let meta_key = format!("snapshot/{}", timestamp);
        let meta_value = serde_json::json!({
            "timestamp": Utc::now(),
            "filename": filename,
            "race_count": races.len(),
            "checksum": checksum,
            "compressed_size": std::fs::metadata(&path)?.len(),
        });
        self.meta_tree
            .insert(meta_key.as_bytes(), serde_json::to_vec(&meta_value)?)?;
        self.meta_tree.flush()?;

        // Clean up old snapshots (keep last 30 days)
        self.cleanup_old_snapshots(30).await?;

        log::info!(
            "Created JSON snapshot: {} with {} races, checksum: {}",
            path.display(),
            races.len(),
            checksum
        );

        Ok(())
    }

    async fn cleanup_old_snapshots(&self, retention_days: u32) -> Result<()> {
        use chrono::Duration;

        let cutoff = Utc::now() - Duration::days(retention_days as i64);
        let mut snapshots_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        snapshots_dir.push(".raceboard");

        if let Ok(entries) = std::fs::read_dir(&snapshots_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if filename.starts_with("races.snapshot.") && filename.ends_with(".json.gz") {
                        // Extract timestamp from filename
                        if let Some(ts_str) = filename
                            .strip_prefix("races.snapshot.")
                            .and_then(|s| s.strip_suffix(".json.gz"))
                        {
                            // Parse timestamp (YYYYMMDD_HHMMSS)
                            if let Ok(dt) = DateTime::parse_from_str(
                                &format!("{} +0000", ts_str.replace('_', " ")),
                                "%Y%m%d %H%M%S %z",
                            ) {
                                if dt.with_timezone(&Utc) < cutoff {
                                    let _ = std::fs::remove_file(&path);
                                    // Also remove checksum file
                                    let mut checksum_path = path.clone();
                                    checksum_path.set_extension("sha256");
                                    let _ = std::fs::remove_file(&checksum_path);
                                    log::info!("Removed old snapshot: {}", filename);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // Persist and load rollout configuration
    pub fn persist_rollout_config(
        &self,
        rollout: &crate::phased_rollout::PhasedRollout,
    ) -> Result<()> {
        let key = b"rollout_config";
        let value = serde_json::to_vec(rollout)?;
        self.meta_tree.insert(key, value)?;
        self.meta_tree.flush()?;
        log::info!("Persisted rollout configuration");
        Ok(())
    }

    pub fn load_rollout_config(&self) -> Result<Option<crate::phased_rollout::PhasedRollout>> {
        if let Some(value) = self.meta_tree.get(b"rollout_config")? {
            match serde_json::from_slice(&value) {
                Ok(rollout) => {
                    log::info!("Loaded rollout configuration from persistence");
                    Ok(Some(rollout))
                }
                Err(e) => {
                    log::error!("Failed to deserialize rollout config: {}", e);
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    pub async fn restore_from_snapshot(&self, snapshot_path: &PathBuf) -> Result<()> {
        use flate2::read::GzDecoder;
        use sha2::{Digest, Sha256};
        use std::io::Read;

        // Verify checksum
        let mut checksum_path = snapshot_path.clone();
        checksum_path.set_extension("sha256");
        if checksum_path.exists() {
            let expected_checksum = std::fs::read_to_string(&checksum_path)?;

            // Read and decompress
            let file = std::fs::File::open(&snapshot_path)?;
            let mut decoder = GzDecoder::new(file);
            let mut json_data = Vec::new();
            decoder.read_to_end(&mut json_data)?;

            // Calculate actual checksum
            let mut hasher = Sha256::new();
            hasher.update(&json_data);
            let actual_checksum = format!("{:x}", hasher.finalize());

            if expected_checksum.trim() != actual_checksum {
                return Err(anyhow::anyhow!(
                    "Checksum mismatch: expected {}, got {}",
                    expected_checksum.trim(),
                    actual_checksum
                ));
            }

            // Deserialize races
            let races: Vec<Race> = serde_json::from_slice(&json_data)?;

            // Import races
            let race_count = races.len();
            for race in races {
                self.store_race(&race).await?;
            }

            log::info!("Restored {} races from snapshot", race_count);

            // Write restore audit record
            let audit = serde_json::json!({
                "action": "restore",
                "snapshot": snapshot_path.display().to_string(),
                "race_count": race_count,
                "timestamp": Utc::now(),
            });
            self.write_audit_record("restore", &audit)?;

            Ok(())
        } else {
            Err(anyhow::anyhow!("Checksum file not found for snapshot"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::ExecutionStats;
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn test_persistence_roundtrip() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let persistence = PersistenceLayer::new(Some(db_path))?;

        let cluster = RaceCluster {
            cluster_id: "test:cluster".to_string(),
            source: "test".to_string(),
            representative_title: "Test Title".to_string(),
            representative_metadata: HashMap::new(),
            stats: ExecutionStats::new_with_default(30),
            member_race_ids: vec!["race1".to_string()],
            member_titles: vec!["Test Title".to_string()],
            member_metadata_history: vec![],
            last_updated: Utc::now(),
            last_accessed: Utc::now(),
        };

        persistence.persist_cluster(&cluster)?;

        let loaded_clusters = persistence.load_clusters()?;
        assert_eq!(loaded_clusters.len(), 1);
        assert!(loaded_clusters.contains_key("test:cluster"));

        Ok(())
    }
}
