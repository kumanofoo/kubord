use log::{info, warn, error};
use chrono::{DateTime, Utc};
use kubord::{
    mqtt::{self, AnomalyKind, AnomalyReport, MQTTMessage},
    GenericError,
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::channel;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct DiscordField {
    name: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordFooter {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordEmbed {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    color: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    fields: Vec<DiscordField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    footer: Option<DiscordFooter>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordWebhook {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    content: String,
    embeds: Vec<DiscordEmbed>,
}

impl DiscordWebhook {
    fn alart(report: AnomalyReport) -> Option<Self>{
        if report.anomalies.is_empty() {
            return None; // No anomalies to report
        }
        let mut embeds = Vec::new();
        for anomaly in report.anomalies {
            let timestamp = DateTime::from_timestamp(anomaly.timestamp, 0).unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let mut embed = DiscordEmbed {
                title: Some(anomaly.host.clone()),
                description: None,
                color: 0x008800, // Green color for normal reports
                timestamp: Some(timestamp),
                fields: Vec::new(),
                footer: None,
            };
            match anomaly.anomaly {
                AnomalyKind::None => {
                    warn!("No anomaly detected for host: {}", anomaly.host);
                    continue; // Skip if no anomaly
                },
                AnomalyKind::AnomalyRtt(rtt_ms, threshold) => {
                    info!("RTT anomaly detected for host {}: RTT {} ms exceeds threshold {} ms", anomaly.host, rtt_ms, threshold);
                    embed.description = Some(format!("RTT: {:.3} ms (Threshold: {:.3} ms)", rtt_ms, threshold));
                    embed.color = 0xFF8800; // Orange color for alerts
                },
                AnomalyKind::PacketLoss(loss_percent) => {
                    info!("Packet loss detected for host {}: {}%", anomaly.host, loss_percent);
                    embed.description = Some(format!("Packet loss: {:.1}%", loss_percent));
                    embed.color = 0xFF8800; // Orange color for alerts
                },
                AnomalyKind::FailToPing(ref error) => {
                    warn!("Failed to ping host {}: {}", anomaly.host, error);
                    embed.description = Some(format!("Ping failed: {}", error));
                    embed.color = 0xFF0000; // Red color for errors
                },
                AnomalyKind::RttData(ref error) => {
                    warn!("RTT data error for host {}: {}", anomaly.host, error);
                    embed.description = Some(format!("RTT historical data error: {}", error));
                    embed.color = 0xFF0000; // Red color for errors
                },
            }
            embeds.push(embed);
        }
        let anomaly_message = DiscordWebhook {
            username: None,
            content: "🚑 Anomaly detected".to_string(),
            embeds: embeds,
        };
        Some(anomaly_message)
    }

    async fn send(message: DiscordWebhook, webhook: String) -> Result<(), GenericError> {
        let client = reqwest::Client::new();
        let body = serde_json::to_string(&message)?;
        let response = client.post(&webhook)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(GenericError::from(format!(
                "Failed to send Discord alert: {}",
                response.status()
            )))
        }

    }
}

#[test]
fn test_discord_webhook_alart() {
    use kubord::mqtt::{Anomaly, AnomalyReport};

    let anomalies = vec![
        Anomaly {
            timestamp: 0,
            host: "www.rtt-anomaly.com".to_string(),
            anomaly: AnomalyKind::AnomalyRtt(100.12345, 50.12345),
        },
        Anomaly {
            timestamp: 1,
            host: "www.packet-loss.com".to_string(),
            anomaly: AnomalyKind::PacketLoss(20.0),
        },
        Anomaly {
            timestamp: 2,
            host: "www.ping-failed.com".to_string(),
            anomaly: AnomalyKind::FailToPing("ping: www.ping-failed.com: Name or service not known".to_string()),
        },
        Anomaly {
            timestamp: 3,
            host: "www.no-historical-data.com".to_string(),
            anomaly: AnomalyKind::RttData("Failed to load historical data".to_string()),
        },
        Anomaly {
            timestamp: 4,
            host: "www.normal.com".to_string(),
            anomaly: AnomalyKind::None,
        },
    ];

    let report = AnomalyReport {
        timestamp: 4,
        anomalies: anomalies,
    };

    let webhook_content = serde_json::to_string(&DiscordWebhook::alart(report).unwrap()).unwrap();
    let expected = concat!(
        "{\"content\":\"🚑 Anomaly detected\",",
        "\"embeds\":[",
            "{\"title\":\"www.rtt-anomaly.com\",",
            "\"description\":\"RTT: 100.123 ms (Threshold: 50.123 ms)\",",
            "\"color\":16746496,",
            "\"timestamp\":\"1970-01-01T00:00:00Z\",",
            "\"fields\":[]},",
            "{\"title\":\"www.packet-loss.com\",",
            "\"description\":\"Packet loss: 20.0%\",",
            "\"color\":16746496,",
            "\"timestamp\":\"1970-01-01T00:00:01Z\",",
            "\"fields\":[]},",
            "{\"title\":\"www.ping-failed.com\",",
            "\"description\":\"Ping failed: ping: www.ping-failed.com: Name or service not known\",",
            "\"color\":16711680,",
            "\"timestamp\":\"1970-01-01T00:00:02Z\",",
            "\"fields\":[]},",
            "{\"title\":\"www.no-historical-data.com\",",
            "\"description\":\"RTT historical data error: Failed to load historical data\",",
            "\"color\":16711680,\"timestamp\":\"1970-01-01T00:00:03Z\",",
            "\"fields\":[]}",
        "]}"
    );
    assert_eq!(webhook_content, expected);

}

#[derive(Debug)]
struct State {
    last_heartbeat: i64, // Unix timestamp in milliseconds
    last_alert: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<(), GenericError> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("mqtt_heartbeat_monitor=info"));

    // Load configuration
    let config = match kubord::load_config() {
        Ok(config) => config,
        Err(why) => {
            eprintln!("Configuration file not found: {:?}", why);
            std::process::exit(1);
        }
    };
    let mqtt_config = match config.mqtt {
        Some(mqtt) => mqtt,
        None => {
            eprintln!("'mqtt' key not found in config.");
            std::process::exit(1);
        }
    };
    let qos = match &mqtt_config.qos {
        Some(q) => q,
        None => &vec![0], // Default QoS to 0 if not specified
    };
    let discord_config = match config.discord {
        Some(discord) => discord,
        None => {
            eprintln!("'discord' key not found in config.");
            std::process::exit(1);
        }
    };
    let heartbeat_config = match config.heartbeat {
        Some(heartbeat) => heartbeat,
        None => {
            eprintln!("'heartbeat' key not found in config.");
            std::process::exit(1);
        }
    };

    // Initialize the MQTT client
    mqtt::initialize_global_topic(&mqtt_config.topics);
    let ping_topic = mqtt::topic("network.ping.report").unwrap().to_string();
    let mqtt_client = mqtt::connect_broker(&mqtt_config).await?;
    let mut subscriber = mqtt::Subscriber::new_from_client(mqtt_client.clone(), qos.clone());

    let shared_state = Arc::new(Mutex::new(State {
        last_heartbeat: Utc::now().timestamp(),
        last_alert: None,
    }));

    let (tx, mut rx) = channel::<MQTTMessage>(128);

    // Spawn a task to handle incoming MQTT messages
    info!("Spawning MQTT message handler task...");
    // This task will receive messages from the MQTT broker and update the shared state.
    // It will also send alerts to Discord if anomalies are detected.
    let webhook = discord_config.webhook.clone();
    let shared_state_clone = Arc::clone(&shared_state);
    let _ = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            info!("mqtt message: {:?}", msg);
            let mut state = shared_state_clone.lock().unwrap();
            // Update last heartbeat
            state.last_heartbeat = Utc::now().timestamp();

            if let Ok(report) = mqtt::AnomalyReport::from_str(&msg.payload) {
                if let Some(webhook_message) = DiscordWebhook::alart(report) {
        		    // Send a anomaly report.
                    info!("Anomaly detected.");
                    let _ = tokio::spawn(DiscordWebhook::send(webhook_message, webhook.clone()));
                } else {
                    // No anomalies.
                    info!("They are alive.");
                }
            } else {
                warn!("Received malformed heartbeat report: {}", msg.payload);
            }
            // Recover from alert if it was active
            if state.last_alert.is_some() {
                info!("Recovered from alert: {}", msg.payload);
                let _ = tokio::spawn(send_discord_alert("✅ Recovered from alert".to_string(), webhook.clone()));
                state.last_alert = None;
            }
        }
    });

    // Spawn a task to monitor heartbeats
    info!("Starting heartbeat monitor task...");
    // This task will check the last heartbeat every 'heartbeat_config.interval_seconds'
    // and send an alert if no heartbeat has been received in the last 'heartbeat_config.timeout_minutes' minutes.
    // It will also snooze alerts for 'heartbeat_config.snooze_alerts_interval_minutes' after the last alert was sent.
    // If a heartbeat is received, it will reset the last heartbeat time.
    let webhook = discord_config.webhook.clone();
    let monitor_state = Arc::clone(&shared_state);
    let _ = tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(heartbeat_config.interval_seconds)).await; // Check every minute
            let mut state = monitor_state.lock().unwrap();
            let current_time = Utc::now().timestamp();
            // Check if the last heartbeat was more than 'heartbeat_config.timeout_seconds' ago
            if current_time - state.last_heartbeat < heartbeat_config.timeout_minutes as i64 * 60 {
                info!("Heartbeat received within the last {} minutes.", heartbeat_config.timeout_minutes);
                continue; // No need to alert
            }

            let should_send_alert = match state.last_alert {
                Some(last_alert) => current_time - last_alert > heartbeat_config.snooze_alerts_interval_minutes as i64 * 60, // If last alert was sent more than 'snooze_alerts_interval_seconds' ago
                None => true, // If no last alert, we should send an alert
            };

            if should_send_alert {
                info!("⚠️ No heartbeat received in the last {} minutes. Sending alert.", heartbeat_config.timeout_minutes);
                // Send alert to Discord
                let alert_message = format!("🚨 No heartbeat received. There may be a network problem.");
                let _ = tokio::spawn(send_discord_alert(alert_message, webhook.clone()));
                // Update state
                state.last_alert = Some(current_time);
            } else {
                info!("Heartbeat lost(Alerts snoozed)");
            }
        }
    });
    // Spawn a task to handle incoming messages
    info!("Subscribing to MQTT topic: {}", ping_topic);
    // This task will subscribe to the MQTT topic and send messages to the channel for processing.
    let _ = subscriber.run(vec![ping_topic.clone()], tx).await?;

    Ok(())
}

async fn send_discord_alert(message: String, webhook: String) -> Result<(), GenericError> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "content": message,
    }).to_string();

    let response = client.post(&webhook)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?;

    if response.status().is_success() {
        info!("📣 Discord alert sent successfully: {}", message);
        Ok(())
    } else {
        error!("Failed to send Discord alert: {}", response.status());
        Err(GenericError::from(format!(
            "Failed to send Discord alert: {}",
            response.status()
        )))
    }
}
