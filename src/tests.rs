#[cfg(test)]
mod tests {
    use crate::models::{Event, Race, RaceState};
    use crate::storage::Storage;

    #[tokio::test]
    async fn test_storage_create_race() {
        let storage = Storage::new();
        let race = Race::new("test".to_string(), "Test Race".to_string());

        let created = storage.create_or_update_race(race.clone()).await;
        assert!(!created.id.is_empty());
        assert_eq!(created.source, "test");
        assert_eq!(created.title, "Test Race");
    }

    #[tokio::test]
    async fn test_storage_get_race() {
        let storage = Storage::new();
        let race = Race::new("test".to_string(), "Test Race".to_string());
        let created = storage.create_or_update_race(race).await;

        let retrieved = storage.get_race(&created.id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn test_storage_update_race() {
        let storage = Storage::new();
        let race = Race::new("test".to_string(), "Test Race".to_string());
        let created = storage.create_or_update_race(race).await;

        let update = crate::models::RaceUpdate {
            source: None,
            title: Some("Updated Title".to_string()),
            state: Some(RaceState::Running),
            started_at: None,
            eta_sec: Some(300),
            progress: Some(50),
            deeplink: None,
            metadata: None,
        };

        let updated = storage.update_race(&created.id, update).await;
        assert!(updated.is_some());
        let updated_race = updated.unwrap();
        assert_eq!(updated_race.title, "Updated Title");
        assert_eq!(updated_race.progress, Some(50));
        assert!(matches!(updated_race.state, RaceState::Running));
    }

    #[tokio::test]
    async fn test_storage_add_event() {
        let storage = Storage::new();
        let race = Race::new("test".to_string(), "Test Race".to_string());
        let created = storage.create_or_update_race(race).await;

        let event = Event::new("test_event".to_string(), None);
        let updated = storage.add_event_to_race(&created.id, event).await;

        assert!(updated.is_some());
        let updated_race = updated.unwrap();
        assert!(updated_race.events.is_some());
        assert_eq!(updated_race.events.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_storage_get_all_races() {
        let storage = Storage::new();

        let race1 = Race::new("test1".to_string(), "Test Race 1".to_string());
        let race2 = Race::new("test2".to_string(), "Test Race 2".to_string());

        storage.create_or_update_race(race1).await;
        storage.create_or_update_race(race2).await;

        let all_races = storage.get_all_races().await;
        assert_eq!(all_races.len(), 2);
    }

    #[tokio::test]
    async fn test_storage_delete_race() {
        let storage = Storage::new();
        let race = Race::new("test".to_string(), "Test Race".to_string());
        let created = storage.create_or_update_race(race).await;

        let deleted = storage.delete_race(&created.id).await;
        assert!(deleted);

        let retrieved = storage.get_race(&created.id).await;
        assert!(retrieved.is_none());
    }
}
