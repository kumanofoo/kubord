// MQTT Subscriber Implementation

//! This module provides an MQTT subscriber that connects to a broker,
//! subscribes to specified topics, and sends received messages to a channel
//! for further processing.
//!
//! ```rust
//! use tokio::sync::mpsc;
//! use kubord::mqtt::{Subscriber, MQTTMessage};
//! use kubord::MqttConfig;
//! use tokio::task;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // MQTT configuration
//!     let config = MqttConfig {
//!         broker: "tcp://localhost:1883".to_string(),
//!         client_id: None,
//!         topics: vec!["test/topic".to_string()],
//!         qos: Some(vec![0]),
//!     };
//!
//!     // Create channel
//!     let (tx, mut rx) = mpsc::channel(32);
//!
//!     // Create subscriber
//!     let mut subscriber = Subscriber::new(&config)?;
//!
//!     // Run subscriber in a separate task
//!     task::spawn(async move {
//!         let _ = subscriber.run(tx).await;
//!     });
//!
//!     // Message receiving loop
//!     while let Some(msg) = rx.recv().await {
//!         println!("Received: topic={}, payload={}", msg.topic, msg.payload);
//!     }
//!     Ok(())
//! }
//! ```

use crate::{GenericError, MqttConfig};
use paho_mqtt as mqtt;
use std::time::Duration;
use std::sync::OnceLock;
use std::collections::HashMap;
use std::fmt;
use tokio::sync::mpsc::Sender;
use log::{debug, info, warn, error};
use futures::stream::StreamExt;
use serde::{Serialize, Deserialize};


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


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AnomalyReport {
    pub timestamp: i64, // Unix timestamp in milliseconds
    pub anomalies: Vec<Anomaly>,
}

impl AnomalyReport {
    pub const MQTT_TOPIC: &str = "message/anomaly/report/rtt";
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

const PUB_CLIENT_ID_PREFIX: &str = "mqtt-pub-";
const SUB_CLIENT_ID_PREFIX: &str = "mqtt-sub-";

pub static TOPICS: OnceLock<HashMap<String, String>> = OnceLock::new();
pub fn initialize_global_topic(topics: &HashMap<String, String>) {
    TOPICS.set(topics.clone()).unwrap();
}
pub fn topic(key: &str) -> Option<String> {
    Some(TOPICS.get()?.get(key)?.clone())
}

#[derive(Debug, Clone)]
pub struct MQTTMessage {
    pub topic: String,
    pub payload: String,
}

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

pub struct Subscriber {
    pub qos: Vec<i32>,
    client: mqtt::AsyncClient,
}

impl Subscriber {
    pub async fn new(config: &MqttConfig) -> Result<Self, GenericError> {
        let client_id = match &config.client_id {
            Some(id) => id,
            None => &format!("{}-{}", SUB_CLIENT_ID_PREFIX, uuid::Uuid::new_v4()),
        };
        let qos = match &config.qos {
            Some(q) => q,
            None => &vec![0], // Default QoS to 0 if not specified
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
            qos: qos.clone(),
            client,
        })
    }

    pub fn new_from_client(
        client: mqtt::AsyncClient,
        qos: Vec<i32>,
    ) -> Self {
        Subscriber {
            qos,
            client,
        }
    }

    pub async fn run(&mut self, topics: Vec<String>, tx: Sender<MQTTMessage>) -> Result<(), GenericError> {
        let mut strm = self.client.get_stream(32);
        self.client.subscribe_many(&topics, &self.qos);

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

pub struct Publisher {
    client: mqtt::AsyncClient,
}

impl Publisher {
    pub async fn new(config: &MqttConfig) -> Result<Self, GenericError> {
        let client_id = match &config.client_id {
            Some(id) => id,
            None => &format!("{}-{}", PUB_CLIENT_ID_PREFIX, uuid::Uuid::new_v4()),
        };
        let qos = match &config.qos {
            Some(q) => q,
            None => &vec![0], // Default Qos to 0 if not specified
        };
        
        info!("Using broker: {}", config.broker);
        info!("Using client ID: {}", client_id);
        info!("Using topics: {:?}", config.topics);
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
}
