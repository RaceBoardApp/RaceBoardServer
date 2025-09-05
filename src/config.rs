use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Settings {
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub http_port: u16,
    pub http_host: String,
    pub grpc_port: u16,
    pub grpc_host: String,
    pub read_only: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub level: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StorageConfig {
    pub max_races: usize,
    pub max_events_per_race: usize,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let config = Config::builder()
            // Start with default values
            .set_default("server.http_port", 7777)?
            .set_default("server.http_host", "127.0.0.1")?
            .set_default("server.grpc_port", 50051)?
            .set_default("server.grpc_host", "127.0.0.1")?
            .set_default("server.read_only", false)?
            .set_default("logging.level", "info")?
            .set_default("storage.max_races", 1000)?
            .set_default("storage.max_events_per_race", 100)?
            // Add config file if it exists
            .add_source(File::with_name("config").required(false))
            // Add environment variables with prefix RACEBOARD_
            // e.g., RACEBOARD_SERVER__HTTP_PORT=8080
            .add_source(Environment::with_prefix("RACEBOARD").separator("__"))
            .build()?;

        config.try_deserialize()
    }

    pub fn http_addr(&self) -> String {
        format!("{}:{}", self.server.http_host, self.server.http_port)
    }

    pub fn grpc_addr(&self) -> String {
        format!("{}:{}", self.server.grpc_host, self.server.grpc_port)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new().expect("Failed to load default settings")
    }
}
