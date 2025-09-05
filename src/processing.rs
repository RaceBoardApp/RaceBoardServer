use crate::prediction::PredictionEngine;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct RaceProcessingRequest {
    pub race_id: String,
    pub race_title: String,
    pub race_source: String,
    pub race_metadata: HashMap<String, String>,
    pub duration: Option<i64>, // Only set when race completes
}

pub struct ProcessingEngine {
    sender: mpsc::Sender<RaceProcessingRequest>,
}

impl ProcessingEngine {
    pub fn new(prediction_engine: Arc<PredictionEngine>) -> Self {
        let (sender, receiver) = mpsc::channel::<RaceProcessingRequest>(100);

        // Spawn background processing task
        tokio::spawn(async move {
            Self::process_queue(receiver, prediction_engine).await;
        });

        Self { sender }
    }

    pub async fn submit_race(&self, request: RaceProcessingRequest) -> Result<(), String> {
        self.sender
            .send(request)
            .await
            .map_err(|e| format!("Failed to submit race for processing: {}", e))
    }

    async fn process_queue(
        mut receiver: mpsc::Receiver<RaceProcessingRequest>,
        prediction_engine: Arc<PredictionEngine>,
    ) {
        while let Some(request) = receiver.recv().await {
            // Clone request data before moving into async block
            let race_id = request.race_id.clone();
            let race_title = request.race_title.clone();

            // Process with timeout
            let engine = prediction_engine.clone();
            let process_task = async move {
                if let Some(duration) = request.duration {
                    // Race completed - update statistics
                    engine
                        .on_race_completed(
                            &request.race_id,
                            &request.race_title,
                            &request.race_source,
                            &request.race_metadata,
                            duration,
                        )
                        .await;
                }
            };

            // Apply timeout
            match timeout(Duration::from_millis(100), process_task).await {
                Ok(_) => {
                    // Successfully processed
                }
                Err(_) => {
                    eprintln!("Processing timeout for race {}: {}", race_id, race_title);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::ClusteringEngine;
    use crate::persistence::PersistenceLayer;

    #[tokio::test]
    async fn test_processing_engine() {
        let clustering_engine = Arc::new(ClusteringEngine::new(100));
        let persistence = Arc::new(PersistenceLayer::new_in_memory().unwrap());
        let prediction_engine = Arc::new(PredictionEngine::new(clustering_engine, persistence));

        let processing_engine = ProcessingEngine::new(prediction_engine);

        let request = RaceProcessingRequest {
            race_id: "test-race".to_string(),
            race_title: "Test Race".to_string(),
            race_source: "test".to_string(),
            race_metadata: HashMap::new(),
            duration: Some(10),
        };

        let result = processing_engine.submit_race(request).await;
        assert!(result.is_ok());

        // Give it time to process
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
