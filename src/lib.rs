//! # Kubord Library
//!
//! This crate provides modules and types for network monitoring, book search, and MQTT/Discord integration.
//!
//! ## Modules
//! - `book`: Book search and library status query via Calil and other APIs.
//! - `mqtt`: MQTT client utilities for subscribing, publishing, and anomaly reporting.
//! - `commands`: Command handling for Discord bot and other integrations.
//!
//! ## Configuration
//! The configuration is loaded from a TOML file (default: `./config.toml`) and includes settings for MQTT, Discord, book search, ping monitor, and heartbeat monitor.
//!
//! ## Example
//! ```toml
//! [mqtt]
//! broker = "tcp://localhost:1883"
//! qos = [0]
//!
//! [discord]
//! channel_id = ["1234567890"]
//! webhook = "https://discord.com/api/webhooks/xxxx/yyyy"
//!
//! [book]
//! libraries = { Tokyo_NDL = "National Diet Library", Tokyo_Pref = "Tokyo Metropolitan Library" }
//!
//! [ping]
//! data_file_prefix = "rtt_data"
//! history_duration_hours = 24
//! ping_interval_minutes = 10
//! anomaly_threshold_factor = 3
//! hosts = ["www.example.com", "8.8.8.8"]
//!
//! [heartbeat]
//! device_id = "rttmon"
//! interval_seconds = 60
//! timeout_minutes = 15
//! snooze_alerts_interval_minutes = 30
//! ```

pub mod book;
pub mod mqtt;
use std::collections::HashMap;
use clap::Parser;
use serde::Deserialize;

/// Generic error type used throughout the crate.
pub type GenericError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub fn progname() -> String {
    let command_path = std::env::args().next().unwrap();
    std::path::Path::new(&command_path).file_name().unwrap().to_str().unwrap().to_string()
}

/// Command-line arguments for configuration file path.
#[derive(Parser)]
#[command(version = env!("CARGO_PKG_VERSION"), about, long_about = None)]
struct Args {
    /// Path to the configuration file (TOML format).
    #[arg(short = 'c', long = "config", default_value = "./config.toml")]
    config_path: String,
}

/// Application configuration loaded from TOML file.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// MQTT broker settings.
    pub mqtt: Option<MqttConfig>,
    /// Discord webhook and channel settings.
    pub discord: Option<DiscordConfig>,
    /// Book search and library settings.
    pub book: Option<BookConfig>,
    /// Ping monitor settings.
    pub ping: Option<PingConfig>,
    /// Heartbeat monitor settings.
    pub heartbeat: Option<HeartbeatConfig>,
}

/// MQTT broker configuration.
#[derive(Debug, Deserialize)]
pub struct MqttConfig {
    /// MQTT broker URI.
    pub broker: String,
    /// Optional MQTT client ID. If not specified, default to {PREFIX} + {UUID}. See mqtt.rs for details. 
    pub client_id: Option<String>,
    /// Optional QoS settings.
    pub qos: Option<Vec<i32>>,
}

impl MqttConfig {
    /// Returns the configured QoS or the default value `[0]`.
    pub fn qos_or_default(&self) -> Vec<i32> {
        self.qos.clone().unwrap_or_else(|| vec![0])
    }
}

type CommandName = String;
type ServiceName = String;

/// Discord webhook and command configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct DiscordConfig {
    /// Discord webhook URL.
    pub webhook: String,
    /// List of Discord channel IDs.
    pub channel_id: Vec<String>,
    /// Command definitions mapped by name.
    pub commands: HashMap<CommandName, ServiceName>,
}

/// Book search configuration.
#[derive(Debug, Deserialize)]
pub struct BookConfig {
    /// Optional service name for book search.
    pub service: Option<String>,
    /// Map of library system IDs to human-readable names.
    pub libraries: HashMap<String, String>,
}

/// Ping monitor configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct PingConfig {
    /// Prefix for RTT data files. The filename will be {data_file_prefix}_{host}.csv.
    pub data_file_prefix: String,
    /// Duration (hours) to keep historical RTT data.
    pub history_duration_hours: u64,
    /// Interval (minutes) between ping checks.
    pub ping_interval_minutes: u64,
    /// Threshold factor for anomaly detection.
    pub anomaly_threshold_factor: f64,
    /// List of hosts to monitor.
    pub hosts: Vec<String>,
    /// Optional device ID for ping monitor.
    pub device_id: Option<String>,
}

impl PingConfig {
    /// Returns the filename for RTT data for a given host.
    pub fn data_filename(&self, host: &str) -> String {
        format!("{}_{}.csv", self.data_file_prefix, host)
    }
}

/// Heartbeat monitor configuration.
#[derive(Debug, Deserialize)]
pub struct HeartbeatConfig {
    /// Device ID for heartbeat monitor.
    pub device_id: String,
    /// Interval (seconds) between heartbeat checks.
    pub interval_seconds: u64,
    /// Timeout (minutes) before alerting on missing heartbeat.
    pub timeout_minutes: u64,
    /// Interval (minutes) to snooze alerts after sending.
    pub snooze_alerts_interval_minutes: u64,
}

/// Loads the application configuration from the specified TOML file.
///
/// # Returns
/// Returns `Ok(Config)` if the configuration is loaded and parsed successfully.
/// Returns `Err(GenericError)` if the file cannot be read or parsed.
///
/// # Example
/// ```rust
/// let config = kubord::load_config().unwrap();
/// ```
pub fn load_config() -> Result<Config, GenericError> {
    let args = Args::parse();
    
    let config_str = std::fs::read_to_string(&args.config_path)?;
    let config = toml::from_str(&config_str)?;
    Ok(config)
}

 /// Loads the application configuration from the specified TOML file.
 ///
 /// # Arguments
 /// * `filename` - The name of the TOML configuration file to load.
 ///
 /// # Returns
 /// Returns `Ok(Config)` if the configuration is loaded and parsed successfully.
 /// Returns `Err(GenericError)` if the file cannot be read or parsed.
 ///
 /// # Example
 /// ```no_run
 /// let config = kubord::load_config_with_filename("my_config.toml").unwrap();
 /// ```
pub fn load_config_with_filename(filename: &str) -> Result<Config, GenericError> {
    let config_path = std::path::Path::new(filename);
    let config_str = std::fs::read_to_string(config_path)?;
    let config = toml::from_str(&config_str)?;
    Ok(config)
}
