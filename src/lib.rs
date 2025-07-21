pub mod book;
pub mod mqtt;
use std::collections::HashMap;
use clap::Parser;
use serde::Deserialize;

pub type GenericError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short = 'c', long = "config", default_value = "./config.json")]
    config_path: String,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub mqtt: Option<MqttConfig>,
    pub discord: Option<DiscordConfig>,
    pub book: Option<BookConfig>,
    pub ping: Option<PingConfig>,
    pub heartbeat: Option<HeartbeatConfig>,
}

#[derive(Debug, Deserialize)]
pub struct MqttConfig {
    pub broker: String,
    pub topics: HashMap<String, String>,
    pub client_id: Option<String>,
    pub qos: Option<Vec<i32>>,
}

impl MqttConfig {
    pub fn qos_or_default(&self) -> Vec<i32> {
        self.qos.clone().unwrap_or_else(|| vec![0])
    }

    pub fn topic(&self, key: &str) -> Option<String> {
        match self.topics.get(key) {
            Some(v) => Some(v.to_string()),
            None => None,
        }
    }
    
    pub fn topic_with_wildcard(&self, key: &str) -> Option<String> {
        match self.topics.get(key) {
            Some(v) => Some(format!("{}/#", v)),
            None => None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiscordConfig {
    pub webhook: String,
    pub channel_id: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BookConfig {
    pub libraries: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PingConfig {
    pub data_file_prefix: String,
    pub history_duration_hours: u64,
    pub ping_interval_minutes: u64,
    pub anomaly_threshold_factor: f64,
    pub hosts: Vec<String>,
}

impl PingConfig {
    pub fn data_filename(&self, host: &str) -> String {
        format!("{}_{}.csv", self.data_file_prefix, host)
    }
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatConfig {
    pub interval_seconds: u64,
    pub timeout_minutes: u64,
    pub snooze_alerts_interval_minutes: u64,
}

pub fn load_config() -> Result<Config, std::io::Error> {
    let args = Args::parse();
    
    let config_str = std::fs::read_to_string(&args.config_path)?;
    let config = serde_json::from_str(&config_str)?;
    Ok(config)
}
