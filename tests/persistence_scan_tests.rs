use chrono::{Duration, Utc};
use tempfile::TempDir;
use RaceboardServer::models::{Race, RaceState};
use RaceboardServer::persistence::{PersistenceLayer, RaceScanFilter, RaceStore};

fn make_race(id: &str, source: &str, offset_secs: i64) -> Race {
    Race {
        id: id.to_string(),
        source: source.to_string(),
        title: format!("race-{}", id),
        state: RaceState::Passed,
        started_at: Utc::now() - Duration::seconds(offset_secs),
        completed_at: Some(Utc::now() - Duration::seconds(offset_secs - 1)),
        duration_sec: Some(1),
        eta_sec: None,
        progress: None,
        deeplink: None,
        metadata: None,
        events: None,
    }
}

#[tokio::test]
async fn test_scan_races_time_order_and_cursor() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test_scan.db");
    let store = PersistenceLayer::new(Some(db_path)).unwrap();

    // Insert races with increasing recency (older first higher offset)
    let r1 = make_race("r1", "cargo", 50);
    let r2 = make_race("r2", "cargo", 40);
    let r3 = make_race("r3", "npm", 30);
    let r4 = make_race("r4", "cargo", 20);
    let r5 = make_race("r5", "npm", 10);

    store.store_race(&r1).await.unwrap();
    store.store_race(&r2).await.unwrap();
    store.store_race(&r3).await.unwrap();
    store.store_race(&r4).await.unwrap();
    store.store_race(&r5).await.unwrap();

    let filter = RaceScanFilter {
        source: None,
        from: None,
        to: None,
        include_events: false,
    };
    let batch1 = store.scan_races(filter.clone(), 2, None).await.unwrap();
    assert_eq!(batch1.items.len(), 2);
    // Should be oldest first by started_at
    let ids1: Vec<_> = batch1.items.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids1, vec!["r1", "r2"]);
    assert!(batch1.next_cursor.is_some());

    let batch2 = store
        .scan_races(filter.clone(), 2, batch1.next_cursor.clone())
        .await
        .unwrap();
    let ids2: Vec<_> = batch2.items.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids2, vec!["r3", "r4"]);

    let batch3 = store
        .scan_races(filter.clone(), 2, batch2.next_cursor.clone())
        .await
        .unwrap();
    let ids3: Vec<_> = batch3.items.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids3, vec!["r5"]);

    // Update a race's started_at and ensure index moves it
    let mut r5_new = r5.clone();
    r5_new.started_at = Utc::now() - Duration::seconds(5);
    store.store_race(&r5_new).await.unwrap();

    let all = store.scan_races(filter.clone(), 10, None).await.unwrap();
    let ids_all: Vec<_> = all.items.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids_all, vec!["r1", "r2", "r3", "r4", "r5"]);
}
