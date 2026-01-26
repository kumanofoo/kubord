// MQTT Subscriber Implementation

//! This module provides MQTT client utilities for subscribing and publishing messages.
//! It includes types and functions for anomaly reporting, topic parsing, and message handling.
//!
//! Main features:
//! - MQTT subscriber and publisher implementations
//! - Anomaly reporting structures
//! - Topic parsing and formatting utilities
//! - Message types for inter-task communication
//!
//! # Example: MQTT Subscriber
//! 
//! ```no_run
//! use tokio::sync::mpsc;
//! use kubord::mqtt::{Subscriber, MQTTMessage};
//! use kubord::MqttConfig;
//! use tokio::task;
//!
//! #[tokio::main]
//! async fn main() {
//!     // MQTT configuration
//!     let config = MqttConfig {
//!         broker: "tcp://localhost:1883".to_string(),
//!         client_id: None,
//!         qos: Some(vec![0]),
//!     };
//!
//!     // Create channel
//!     let (tx, mut rx) = mpsc::channel(32);
//!
//!     // Create subscriber
//!     let mut subscriber = Subscriber::new(&config).await.unwrap();
//!
//!     // Run subscriber in a separate task
//!     task::spawn(async move {
//!         let _ = subscriber.run(vec!["test/topic".to_string()], tx).await;
//!     });
//!
//!     // Message receiving loop
//!     while let Some(msg) = rx.recv().await {
//!         println!("Received: topic={}, payload={}", msg.topic, msg.payload);
//!     }
//! }
//! ```
//! 
//! # Example: MQTT Publisher
//!
//! ```no_run
//! use kubord::mqtt::{Publisher};
//! use kubord::MqttConfig;
//!
//! #[tokio::main]
//! async fn main() {
//!     // MQTT configuration
//!     let config = MqttConfig {
//!         broker: "tcp://localhost:1883".to_string(),
//!         client_id: None,
//!         qos: Some(vec![0])
//!     };
//! 
//!     // Create publisher
//!     let publisher = Publisher::new(&config).await.unwrap();
//! 
//!     // Publish message
//!     publisher.publish("test/topic", "Hello MQTT!").await;
//! }
//! ```

use crate::{GenericError, MqttConfig};
use paho_mqtt as mqtt;
use std::time::Duration;
use std::fmt;
use tokio::sync::mpsc::Sender;
use log::{debug, info, warn, error};
use futures::stream::StreamExt;
use serde::{Serialize, Deserialize};


/// Represents the type of anomaly detected in network monitoring.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum AnomalyKind {
    AnomalyRtt(f64, f64), // RTT value and threshold
    PacketLoss(f64), // Packet loss percentage
    FailToPing(String),
    RttData(String),
    None, // No anomaly detected
}

impl std::fmt::Display for AnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnomalyKind::AnomalyRtt(rtt, threshold) => {
                write!(f, "Anomaly detected: RTT ({:.2} ms) exceeds threshold ({:.2} ms)", rtt, threshold)
            }
            AnomalyKind::PacketLoss(loss) => write!(f, "Packet loss detected: {:.2}%", loss),
            AnomalyKind::FailToPing(err) => write!(f, "Ping failed: {}", err),
            AnomalyKind::RttData(err) => write!(f, "RTT historical data error: {}", err),
            AnomalyKind::None => write!(f, "No anomaly detected"),
        }
    }
}


/// Contains information about a detected anomaly for a specific host and time.
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct Anomaly {
    pub timestamp: i64, // Unix timestamp in milliseconds
    pub host: String, // Hostname
    pub anomaly: AnomalyKind, // Optional error message if ping fails
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Convert timestamp to a human-readable format
        let datetime_string = match chrono::DateTime::from_timestamp(self.timestamp, 0) {
            Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "Invalid timestamp".to_string(),
        };
        writeln!(f, "[ANOMALY REPORT]")?;
        writeln!(f, "- Timestamp: {}", datetime_string)?;
        writeln!(f, "- Host: {}", self.host)?;
        write!(f, "- Anomaly: {}", self.anomaly)
    }
}


/// Aggregates multiple anomalies detected at a specific timestamp.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AnomalyReport {
    pub timestamp: i64, // Unix timestamp in milliseconds
    pub anomalies: Vec<Anomaly>,
}

impl AnomalyReport {
    pub fn new() -> Self {
        AnomalyReport {
            timestamp: chrono::Utc::now().timestamp_millis(),
            anomalies: Vec::new(),
        }
    }

    pub fn from_str(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn add_anomaly(&mut self, anomaly: Anomaly) {
        self.anomalies.push(anomaly);
    }
}

impl std::fmt::Display for AnomalyReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}


use jma::amedas::{Amedas, AmedasData, AmedasStation};
type Kanji = String;
type English = String;
type Code = String;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MQTTAmedas {
    pub station: (Kanji, English, Code),
    pub latest_time: String,
    pub data: AmedasData,
}

impl MQTTAmedas {
    pub fn new(amedas: &Amedas, amedas_station: &str, station_info: &AmedasStation) -> Self {
        let latest = AmedasData::from(&amedas.get_latest_data().unwrap());
        MQTTAmedas {
            station: (
                station_info.kanji_name.to_string(),
                station_info.english_name.to_string(),
                amedas_station.to_string(),
            ),
            latest_time: amedas.latest_time.clone(),
            data: latest.clone(),
        }
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

impl Mqtt for MQTTAmedas {
    fn topic(&self) -> String {
        Topic::Sensor { location: "amedas".to_string(), device_id: self.station.2.clone() }.to_string()
    }
    fn payload(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}


const PUB_CLIENT_ID_PREFIX: &str = "mqtt-pub";
const SUB_CLIENT_ID_PREFIX: &str = "mqtt-sub";

/// Errors that can occur when parsing MQTT topic strings.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseTopicError {
    UnknownPrefix,
    InvalidFormat,
}

impl std::fmt::Display for ParseTopicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseTopicError::UnknownPrefix => write!(f, "Unknown topic prefix"),
            ParseTopicError::InvalidFormat => write!(f, "Invalid topic format"),
        }
    }
}

impl std::error::Error for ParseTopicError {}

/// Enum representing a boolean value intended for use as part of an MQTT topic.
///
/// This enum introduces the `Any` variant to represent a wildcard for
/// subscription purposes, in addition to the standard true/false values.
pub enum TopicBool {
    True, False, Any
}

impl std::fmt::Display for TopicBool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	match self {
	    TopicBool::True => write!(f, "true"),
	    TopicBool::False => write!(f, "false"),
	    TopicBool::Any => write!(f, "+"),
	}
    }
}

impl std::str::FromStr for TopicBool {
    type Err = ParseTopicError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "true" | "True" | "TRUE"  => Ok(TopicBool::True),
            "false" | "False" | "FALSE" => Ok(TopicBool::False),
            _ => Err(ParseTopicError::InvalidFormat),
        }
    }
}


/// Represents MQTT topic formats used in the application.
///
/// - `Command`: Used to send commands to a device (e.g., turning a light on/off).
/// - `Response`: Used to send responses to a command (e.g., device returns its status).
/// - `Sensor`: Used to periodically publish sensor measurement results (e.g., temperature readings).
/// - `Monitor`: Used to monitor and publish the status of a device or system (e.g., network health).
pub enum Topic {
    /// Topic for sending commands or requests to a device or server.
    /// Example: Used to instruct a lighting device to turn on or off.
    Command { service: String, session_id: String },

    /// Topic for sending responses to a command or requests.
    /// Example: Used by a device to reply with its status after receiving a command.
    Response { service: String, session_id: String, is_last: TopicBool},

    /// Topic for publishing periodic sensor measurement results.
    /// Example: Used by a temperature sensor to publish readings every 10 minutes.
    Sensor { location: String, device_id: String },

    /// Topic for monitoring and publishing the status of a device or system.
    /// Example: Used to publish network status or alerts when an abnormal condition is detected.
    Monitor { device_id: String },
}

impl Topic {
    const COMMANDS: [&str; 4] = ["command", "response", "sensor", "monitor"];
    pub const ANY: &str = "+";
    pub fn all_command() -> Self {
        Topic::Command {
            service: "+".to_string(),
            session_id: "#".to_string(),
        }
    }
    pub fn all_response() -> Self {
        Topic::Response {
            service: "+".to_string(),
            session_id: "+".to_string(),
	        is_last: TopicBool::Any,
        }
    }
    pub fn all_sensor() -> Self {
        Topic::Sensor {
            location: "+".to_string(),
            device_id: "#".to_string(),
        }
    }
    pub fn all_monitor() -> Self {
        Topic::Monitor {
            device_id: "+".to_string(),
        }
    }
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Topic::Command { service, session_id } =>
                write!(f, "command/{}/{}", service, session_id),
            Topic::Response { service, session_id, is_last } =>
                write!(f, "response/{}/{}/{}", service, session_id, is_last),
            Topic::Sensor { location, device_id } =>
                write!(f, "sensor/{}/{}", location, device_id),
            Topic::Monitor { device_id } =>
                write!(f, "monitor/{}", device_id),
        }
    }
}

impl std::str::FromStr for Topic {
    type Err = ParseTopicError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        match parts.as_slice() {
            ["command", service, session_id] =>
                Ok(Topic::Command {service: service.to_string(), session_id: session_id.to_string()}),
            ["response", service, session_id, is_last_str] => {
                let is_last = match TopicBool::from_str(is_last_str) {
                    Ok(topic_bool) => topic_bool,
                    Err(_) => return Err(ParseTopicError::InvalidFormat),
                };
                Ok(Topic::Response {service: service.to_string(), session_id: session_id.to_string(), is_last})
	    },
            ["sensor", location, device_id] =>
                Ok(Topic::Sensor {
                    location: location.to_string(),
                    device_id: device_id.to_string()
                }),
            ["status", device_id] =>
                Ok(Topic::Monitor {
                    device_id: device_id.to_string()
                }),
            //[prefix, ..] if *prefix != "command" && *prefix != "response" && *prefix != "sensor" && *prefix != "monitor" =>
            [prefix, ..] if !Topic::COMMANDS.contains(prefix) =>
                Err(ParseTopicError::UnknownPrefix),
            _ => Err(ParseTopicError::InvalidFormat),
        }
    }
}

/// Represents a received MQTT message with topic and payload.
pub trait Mqtt {
    fn topic(&self) -> String;
    fn payload(&self) -> String;
}

#[derive(Debug, Clone)]
pub struct MQTTMessage {
    pub topic: String,
    pub payload: String,
}

impl Mqtt for MQTTMessage {
    fn topic(&self) -> String {
        self.topic.clone()
    }

    fn payload(&self) -> String {
        self.payload.clone()
    }
}

/// Connects to the MQTT broker using the provided configuration.
/// Returns an asynchronous MQTT client on success.
pub async fn connect_broker(config: &MqttConfig) -> Result<mqtt::AsyncClient, GenericError> {
    let client_id = match &config.client_id {
        Some(id) => id,
        None => &format!("{}-{}", SUB_CLIENT_ID_PREFIX, uuid::Uuid::new_v4()),
    };

    info!("Using broker: {}", config.broker);
    info!("Using client ID: {}", client_id);

    // Create the MQTT client with the specified options.
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(config.broker.to_string())
        .client_id(client_id.clone())
        .finalize();
    let client = mqtt::AsyncClient::new(create_opts)?;
    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .automatic_reconnect(
        Duration::from_secs(1),
        Duration::from_secs(30)
        )
        .finalize();
    client.connect(conn_opts).await?;
    info!("Connected to MQTT broker: {}", config.broker);
    Ok(client)
}

/// MQTT subscriber for receiving messages from specified topics.
pub struct Subscriber {
    pub qos: i32,
    client: mqtt::AsyncClient,
}

impl Subscriber {
    /// Creates a new MQTT subscriber and connects to the broker.
    pub async fn new(config: &MqttConfig) -> Result<Self, GenericError> {
        let client_id = match &config.client_id {
            Some(id) => id,
            None => &format!("{}-{}", SUB_CLIENT_ID_PREFIX, uuid::Uuid::new_v4()),
        };
        let qos = match config.qos {
            Some(q) => q,
            None => 0, // Default QoS to 0 if not specified
        };

        info!("Using broker: {}", config.broker);
        info!("Using client ID: {}", client_id);
        info!("Using QoS: {:?}", qos);

        // Create the MQTT client with the specified options.
        let create_opts = mqtt::CreateOptionsBuilder::new()
            .server_uri(config.broker.to_string())
            .client_id(client_id.clone())
            .finalize();
        let client = mqtt::AsyncClient::new(create_opts)?;
        // Define the set of options for the connection.
        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .keep_alive_interval(Duration::from_secs(30))
            .clean_session(true)
            .finalize();
        // Make the connection to the broker
        client.connect(conn_opts).await?;

        Ok(Subscriber {
            qos,
            client,
        })
    }

    pub fn new_from_client(
        client: mqtt::AsyncClient,
        qos: i32,
    ) -> Self {
        Subscriber {
            qos,
            client,
        }
    }

    /// Runs the subscriber, listening to topics and forwarding messages to the provided channel.
    pub async fn run(&mut self, topics: Vec<String>, tx: Sender<MQTTMessage>) -> Result<(), GenericError> {
        let mut strm = self.client.get_stream(32);
	let qos = vec![self.qos; topics.len()];
        self.client.subscribe_many(&topics, &qos);

        info!("start subscribing...");
        while let Some(msg_opt) = strm.next().await {
            if let Some(msg) = msg_opt {
                debug!("{}: {}", msg.topic(), msg.payload_str());
                tx.send(MQTTMessage {
                    topic: msg.topic().to_string(),
                    payload: msg.payload_str().to_string(),
                }).await.expect("Failed to send");
            } else {
                while let Err(err) = self.client.reconnect().await {
                    error!("Error reconnecting: {:?}", err);
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                }
            }
        }
        info!("Subscriber exited.");
        Ok(())
    }
}

/// MQTT publisher for sending messages to specified topics.
pub struct Publisher {
    client: mqtt::AsyncClient,
}

impl Publisher {
    /// Creates a new MQTT publisher and connects to the broker.
    pub async fn new(config: &MqttConfig) -> Result<Self, GenericError> {
        let client_id = match &config.client_id {
            Some(id) => id,
            None => &format!("{}-{}", PUB_CLIENT_ID_PREFIX, uuid::Uuid::new_v4()),
        };
        let qos = match config.qos {
            Some(q) => q,
            None => 0, // Default Qos to 0 if not specified
        };
        
        info!("Using broker: {}", config.broker);
        info!("Using client ID: {}", client_id);
        info!("Using QoS: {:?}", qos);

        // Create the MQTT client with the specified options.
        let create_opts = mqtt::CreateOptionsBuilder::new()
            .server_uri(config.broker.to_string())
            .client_id(client_id.clone())
            .finalize();
        let client = mqtt::AsyncClient::new(create_opts)?;
        
        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .keep_alive_interval(Duration::from_secs(20))
            .clean_session(true)
            .automatic_reconnect(
                Duration::from_secs(1),
                Duration::from_secs(30)
            )
            .finalize();
        
        client.connect(conn_opts).await?;
        info!("Connected to MQTT broker: {}", config.broker);
        
        Ok(Publisher {
            client,
        })
    }
    
    pub fn new_from_client(client: mqtt::AsyncClient) -> Self {
        Publisher {
            client,
        }
    }

    /// Publishes a message to the specified topic.
    pub async fn publish(&self, topic: &str, payload: &str) {
        let msg = mqtt::Message::new(topic, payload.as_bytes(), 0);
        debug!("topic: {}, payload: {}", msg.topic(), std::str::from_utf8(msg.payload()).unwrap());
        match self.client.publish(msg).await {
            Ok(_) => debug!("Successfully published to MQTT: Topic='{}', Payload='{}'", topic, payload),
            Err(why) => {
                warn!("Failed to publish to MQTT: {:?}", why);
            },
        }
    }

    /// Publishes a retained message to the specified topic.
    pub async fn publish_retained_message(&self, topic: &str, payload: &str) {
        let msg = mqtt::Message::new_retained(topic, payload.as_bytes(), 0);
        debug!("topic: {}, payload: {}", msg.topic(), std::str::from_utf8(msg.payload()).unwrap());
        match self.client.publish(msg).await {
            Ok(_) => debug!("Successfully published to MQTT: Topic='{}', Payload='{}', Retained", topic, payload),
            Err(why) => {
                warn!("Failed to publish to MQTT: {:?}", why);
            },
        }
    }

    pub async fn publish_mqtt<T: Mqtt>(&self, mqtt: &T) {
        let topic = mqtt.topic();
        let payload = mqtt.payload();
        self.publish(&topic, &payload).await;
    }
}
