use crate::adapter_status::AdapterRegistry;
use crate::monitoring::{AlertSystem, DataLayerMetrics, MonitoringSystem};
use crate::persistence::PersistenceLayer;
use crate::prediction::PredictionEngine;
use crate::processing::ProcessingEngine;
use crate::rebuild::DoubleBufferClusters;
use crate::rebuild_trigger::RebuildTrigger;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
    pub prediction_engine: Arc<PredictionEngine>,
    pub processing_engine: Arc<ProcessingEngine>,
    pub rebuild_clusters: Arc<DoubleBufferClusters>,
    pub rebuild_trigger: Arc<RebuildTrigger>,
    pub persistence: Arc<PersistenceLayer>,
    pub monitoring: Arc<MonitoringSystem>,
    pub alert_system: Arc<AlertSystem>,
    pub data_layer_metrics: Option<Arc<DataLayerMetrics>>, 
    pub adapter_registry: Arc<AdapterRegistry>,
    pub read_only: bool,
    pub legacy_json_fallback_enabled: bool,
}
