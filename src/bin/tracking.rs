use log::{info, warn, error};
use std::collections::HashMap;
use std::sync::OnceLock;
use clap::Parser;
use chrono::{Utc, DateTime, Local, Datelike, FixedOffset, ParseError};
use tokio::time::{Duration, sleep};
use tokio::task::JoinHandle;
use kubord::progname;
use kubord::mqtt::{connect_broker, Publisher, Subscriber, Topic, TopicBool, MQTTMessage};
use kubord::commands::tracking::{
    TrackingItem,
    TrackingRecord,
    Carrier,
    SubCommand,
    SubCommandResponse,
};
use reqwest::{RequestBuilder, StatusCode};

const DEFAULT_MAX_RETRIES: u32 = 10;
static MAX_RETRIES: OnceLock<u32> = OnceLock::new();

async fn backoff(atempt: u32) {
    let wait = Duration::from_secs((2u64.pow(atempt)).min(900));
    sleep(wait).await;
}

async fn send_with_retry(request: &RequestBuilder) -> Result<String, String> {
    let max_retries = *MAX_RETRIES.get().unwrap_or_else(|| {
        warn!("MAX_RETRIES is uninitialized, so use {}.", DEFAULT_MAX_RETRIES);
        &DEFAULT_MAX_RETRIES
    });
    let mut attempt = 0;
    loop {
        let request_clone = request.try_clone().unwrap();
        let result = request_clone.send().await;
        match result {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    match resp.text().await {
                        Ok(text) => return Ok(text),
                        Err(why) => return Err(why.to_string()),
                    }
                }
                else if status.is_server_error() ||
                    status == StatusCode::TOO_MANY_REQUESTS ||
                    attempt < max_retries {
                    attempt += 1;
                    backoff(attempt).await;
                    continue;
                }

                return Err(resp.status().to_string());
            },
            Err(err) => {
                if (err.is_connect() || err.is_timeout() || err.is_request()) && attempt < max_retries {
                    attempt += 1;
                    warn!("Failed to send a request. Retrying... ({} <= {}): {}", attempt, max_retries, err);
                    backoff(attempt).await;
                    continue;
                }

                return Err(err.to_string());
            },
        }
    }
}

async fn fetch_delivery_status_jp(number: &str) -> Result<TrackingItem, String> {
    let url = Carrier::Jp.query_url(number);
    let client = reqwest::Client::new();
    let request = client.get(&url);
    let body = send_with_retry(&request).await?;
    let record_selector = scraper::Selector::parse("tr").unwrap();
    let document = scraper::Html::parse_document(&body);
    let records = document.select(&record_selector);

    let data_selector = scraper::Selector::parse("td").unwrap();
    let mut tracking = Vec::new();
    for tr in records {
        let mut tds = tr.select(&data_selector);
        if let Some(td) = tds.next() {
            if let Some(text) = td.text().next() {
                let timestamp = match DateTime::parse_from_str(&format!("{} +0900", text), "%Y/%m/%d %H:%M %z") {
                    Ok(datetime) => {
                        datetime.timestamp()
                    },
                    Err(_) => continue,
                };
                let activity = match tds.next() {
                    Some(td) => match td.text().next() {
                        Some(text) => text.to_string(),
                        None => continue,
                    }
                    None => continue,
                };
                let _detail = match tds.next() {
                    Some(_) => (),
                    None => continue,
                };
                let office = match tds.next() {
                    Some(td) => match td.text().next() {
                        Some(text) => Some(text.to_string()),
                        None => None,
                    },
                    None => continue,
                };
                tracking.push(TrackingRecord {timestamp, activity, office});
            }
        }
    }
    
    Ok(TrackingItem {
        item_number: number.to_string(),
        alias: None,
        carrier: Carrier::Jp,
        timestamp: Utc::now().timestamp(),
        log: tracking,
    })
}



/// Attempts to infer the year for a datetime string that is missing the year component,
/// based on a reference date and time.
///
/// This function is typically used when parsing relative dates (e.g., "11/01 10:30")
/// where the year is ambiguous. The inference logic attempts to determine whether the
/// resulting date should be in the current year (`reference.year()`) or the previous/next year,
/// usually aiming for a date that is relatively close to the `reference` date,
/// specifically comparing against one month from the `reference` date.
///
/// **Note:** This function hardcodes the timezone offset to `+0900` during parsing,
/// which implies an assumption about the target timezone (JST/KST).
///
/// # Arguments
///
/// * `datetime`: A string slice containing the datetime components (e.g., month, day, time)
///   **excluding** the year.
/// * `format`: A string slice defining the format of `datetime` using `chrono`'s
///   formatting codes, **excluding** `%Y` (year) and `%z` (timezone offset).
/// * `reference`: A `DateTime<Local>` used as the basis for inferring the missing year.
///
/// # Returns
///
/// A `Result<DateTime<FixedOffset>, ParseError>`.
/// * `Ok(DateTime<FixedOffset>)`: The successfully parsed datetime with the inferred year.
/// * `Err(ParseError)`: If the datetime string, even with the inferred year,
///   could not be parsed according to the format.
///
/// # Example (Illustrative)
///
/// ```
/// use chrono::{Local, TimeZone, Datelike, Timelike};
/// # use chrono::{DateTime, FixedOffset, ParseError, Months};
/// # fn infer_year(datetime: &str, format: &str, reference: DateTime<Local>) -> Result<DateTime<FixedOffset>, ParseError> {
/// #     let format_added_year = &format!("{} %Y %z", format);
/// #     let reference_year = reference.year();
/// #     let next_month = reference + Months::new(1);
/// #     let inferred_year = if reference_year == next_month.year() {
/// #         let datetime_with_this_year = DateTime::parse_from_str(
/// #             &format!("{} {} +0900", datetime, reference_year),
/// #             format_added_year
/// #         )?;
/// #         if datetime_with_this_year > next_month {
/// #             reference_year - 1
/// #         } else {
/// #             reference_year
/// #         }
/// #     } else {
/// #         let next_year = next_month.year();
/// #         let datetime_with_next_year = DateTime::parse_from_str(
/// #             &format!("{} {} +0900", datetime, next_year),
/// #             format_added_year
/// #         )?;
/// #         if datetime_with_next_year < next_month {
/// #             next_year
/// #         } else {
/// #             reference_year
/// #         }
/// #     };
/// #     DateTime::parse_from_str(
/// #         &format!("{} {} +0900", datetime, inferred_year),
/// #         format_added_year
/// #     )
/// # }
/// // Assume current date is 2025-01-15 12:00:00 +09:00 (Local)
/// let reference_dt = Local.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap();
///
/// // Case 1: Target date is in the same month/year
/// // "01/20 10:00" -> 2025-01-20
/// let result_1 = infer_year("01/20 10:00", "%m/%d %H:%M", reference_dt).unwrap();
/// assert_eq!(result_1.year(), 2025);
///
/// // Case 2: Target date is in the *past* relative to the reference,
/// // but within one month's "lookahead window", so it rolls back to the previous year.
/// // Reference is 2025-01-15. next_month is 2025-02-15.
/// // Parsing "12/25 10:00" with 2025 (2025-12-25) is > 2025-02-15, so it infers 2024.
/// // This specific logic only applies if the next month is in the *same* year.
/// let reference_dt_mid_year = Local.with_ymd_and_hms(2025, 7, 15, 12, 0, 0).unwrap();
/// // "06/25 10:00" -> 2025-06-25 (same year)
/// let result_2 = infer_year("06/25 10:00", "%m/%d %H:%M", reference_dt_mid_year).unwrap();
/// assert_eq!(result_2.year(), 2025);
///
/// // Case 3: Year wrap-around (reference is near year-end)
/// // Reference is 2025-12-15. next_month is 2026-01-15.
/// let reference_dt_year_end = Local.with_ymd_and_hms(2025, 12, 15, 12, 0, 0).unwrap();
///
/// // "01/05 10:00" -> 2026-01-05 (next year)
/// let result_3 = infer_year("01/05 10:00", "%m/%d %H:%M", reference_dt_year_end).unwrap();
/// assert_eq!(result_3.year(), 2026);
///
/// // "12/20 10:00" -> 2025-12-20 (same year)
/// let result_4 = infer_year("12/20 10:00", "%m/%d %H:%M", reference_dt_year_end).unwrap();
/// assert_eq!(result_4.year(), 2025);
/// ```
fn infer_year(datetime: &str, format: &str, reference: DateTime<Local>) -> Result<DateTime<FixedOffset>, ParseError> {
    let format_added_year = &format!("{} %Y %z", format);
    let reference_year = reference.year();
    let next_month = reference + chrono::Months::new(1);
    let inferred_year = if reference_year == next_month.year() {
        let datetime_with_this_year = DateTime::parse_from_str(
            &format!("{} {} +0900", datetime, reference_year),
            format_added_year
        )?;
        if datetime_with_this_year > next_month {
            reference_year - 1
        } else {
            reference_year
        }
    } else {
        let next_year = next_month.year();
        let datetime_with_next_year = DateTime::parse_from_str(
            &format!("{} {} +0900", datetime, next_year),
            format_added_year
        )?;
        if datetime_with_next_year < next_month {
            next_year
        } else {
            reference_year
        }
    };
    DateTime::parse_from_str(
        &format!("{} {} +0900", datetime, inferred_year),
        format_added_year
    )
}

async fn fetch_delivery_status_sagawa(number: &str) -> Result<TrackingItem, String> {
    const DATETIME_FORMAT: &str = "%m/%d %H:%M";
    let url = Carrier::Sagawa.query_url(number);
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0")
        .build().unwrap();
    
    let request = client.get(&url);
    let body = send_with_retry(&request).await?;

    let table_selector = scraper::Selector::parse("#detail1 table").unwrap();
    let document = scraper::Html::parse_document(&body);
    let mut tables = document.select(&table_selector);
    let _metainfo = tables.next();

    let record_selector = scraper::Selector::parse("tr").unwrap();
    let records = tables.next().unwrap().select(&record_selector);
    let item_selector = scraper::Selector::parse("td").unwrap();
    let mut tracking = Vec::new();
    for tr in records {
        let mut items = tr.select(&item_selector);
        let activity = match items.next() {
            Some(item) => match item.text().next() {
                Some(text) => text.trim().to_string(),
                None => continue,
            },
            None => continue,
        };
        let timestamp = match items.next() {
            Some(item) => match item.text().next() {
                Some(text) => {
                    match infer_year(text, DATETIME_FORMAT, Local::now()) {
                        Ok(dt) => {
                            dt.timestamp()
                        },
                        Err(why) => {
                            warn!("Failed to parse datetime: {}", why);
                            continue;
                        }
                    }
                },
                None => continue,
            },
            None => continue,
        };
        let office = match items.next() {
            Some(item) => match item.text().next() {
                Some(text) => Some(text.trim().to_string()),
                None => None,
            },
            None => continue,
        };
        tracking.push(TrackingRecord {timestamp, activity, office});
    }

    Ok(TrackingItem {
        item_number: number.to_string(),
        alias: None,
        carrier: Carrier::Sagawa,
        timestamp: Utc::now().timestamp(),
        log: tracking,
    })
}

async fn fetch_delivery_status_yamato(number: &str) -> Result<TrackingItem, String> {
    const DATETIME_FORMAT: &str = "%m月%d日 %H:%M";
    let url = "https://toi.kuronekoyamato.co.jp/cgi-bin/tneko";
    let mut param = HashMap::new();
    param.insert("number00", "1");
    param.insert("number01", number);
    let client = reqwest::Client::new();
    let request = client.post(url).form(&param);
    let body = send_with_retry(&request).await?;
    
    let record_selector = scraper::Selector::parse(".tracking-invoice-block-detail > ol > li").unwrap();
    let document = scraper::Html::parse_document(&body);
    let records = document.select(&record_selector);

    let item_selector = scraper::Selector::parse(".item").unwrap();
    let date_selector = scraper::Selector::parse(".date").unwrap();
    let name_selector = scraper::Selector::parse(".name").unwrap();
    let mut tracking = Vec::new();
    for li in records {
        let mut items = li.select(&item_selector);
        let activity = match items.next() {
            Some(item) => match item.text().next() {
                Some(text) => text.to_string(),
                None => continue,
            },
            None => continue,
        };
        let mut dates = li.select(&date_selector);
        let timestamp = match dates.next() {
            Some(date) => match date.text().next() {
                Some(text) => {
                    match infer_year(text, DATETIME_FORMAT, Local::now()) {
                        Ok(dt) => {
                            dt.timestamp()
                        },
                        Err(why) => {
                            warn!("Failed to parse datetime: {}", why);
                            continue;
                        },
                    }
                },
                None => continue,
            },
            None => continue,
        };
        let mut names = li.select(&name_selector);
        let office = match names.next() {
            Some(name) => match name.text().next() {
                Some(text) => Some(text.to_string()),
                None => None,
            },
            None => continue,
        };
        tracking.push(TrackingRecord {timestamp, activity, office});
    }
    
    Ok(TrackingItem {
        item_number: number.to_string(),
        alias: None,
        carrier: Carrier::Yamato,
        timestamp: Utc::now().timestamp(),
        log: tracking,
    })
}

#[derive(Parser)]
#[command(version = env!("CARGO_PKG_VERSION"), about, long_about = None)]
struct Args {
    #[arg(short = 'c', long = "config", default_value = "./config.toml")]
    config_path: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let args = Args::parse();
    let config = match kubord::load_config_with_filename(&args.config_path) {
            Ok(config) => config,
            Err(why) => {
                error!("Configuration file error: {}", why);
                std::process::exit(1);
            }
    };

    // Default Parameters
    let mut device_id = progname();
    let mut tracking_interval_seconds: u64 = 60*60; // 1 hour
    let mut retry_interval_seconds: u64 = 120; // 2 minutes
    let mut max_retries: u32 = DEFAULT_MAX_RETRIES;
    if let Some(tracking) = config.tracking {
        if let Some(x) = tracking.device_id {
            device_id = x;
        }
        if let Some(x) = tracking.tracking_interval_seconds {
            tracking_interval_seconds = x;
        }
        if let Some(x) = tracking.retry_interval_seconds {
            retry_interval_seconds = x;
        }
        if let Some(x) = tracking.max_retries {
            max_retries = x;
        }
    }
    if let Err(e) = MAX_RETRIES.set(max_retries) {
        error!("MAX_RETRIES is already set?: {}", e);
    }
    info!("device_id: {device_id}");
    info!("tracking_interval_seconds: {tracking_interval_seconds}");
    info!("retry_interval_seconds: {retry_interval_seconds}");
    info!("max_retries: {max_retries}");
    
    let mqtt_config = match config.mqtt {
        Some(mqtt) => mqtt,
        None => {
        error!("No MQTT configuration.");
            std::process::exit(1);
        }
    };
    let qos = match mqtt_config.qos {
        Some(q) => q,
        None => 0, // Default QoS to 0 if not specified
    };

    let mqtt_client = match connect_broker(&mqtt_config).await {
        Ok(client) => {
            info!("Connected to MQTT broker: {}", mqtt_config.broker);
            client
        },
        Err(why) => {
            error!("Failed to connect broker: {}", why);
            std::process::exit(1);
        }
    };
    
    let mut subscriber = Subscriber::new_from_client(mqtt_client.clone(), qos);
    let publisher = Publisher::new_from_client(mqtt_client.clone());
    let command_topic = vec![Topic::Command {
        service: device_id.clone(),
        session_id: Topic::ANY.to_string(),
    }.to_string()];
    let (mqtt_tx, mut mqtt_rx) = tokio::sync::mpsc::channel::<MQTTMessage>(128);
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<MQTTMessage>(128);

    info!("Tracking started.");
    tokio::select! {
        // Subscriber
        _ = subscriber.run(command_topic, mqtt_tx) => {
            info!("Subscriber task exited.");
        },
        // Publisher
        _ = tokio::spawn(async move {
            while let Some(message) = response_rx.recv().await {
                publisher.publish(&message.topic, &message.payload).await;
                info!("publish: {} - {}", message.topic, message.payload);
            }
        }) => {
            info!("Publisher task exited.");
        },
        // Command Handler
        _ = tokio::spawn(async move {
            info!("Waiting for Command...");
            let mut items = HashMap::<String, JoinHandle<_>>::new();
            let mut aliases = HashMap::<String, String>::new();
            while let Some(mqtt_message) = mqtt_rx.recv().await {
                info!("{} - {}", mqtt_message.topic, mqtt_message.payload);
                let session_id = match mqtt_message.topic.parse::<Topic>() {
                    Ok(topic) => match topic {
                        Topic::Command {service: _, session_id} => session_id,
                        _ => {
                            warn!("Not command message: {}", mqtt_message.topic);
                            continue;
                        }
                    },
                    Err(why) => {
                        warn!("Failed to extract session ID from MQTT topic: {}: {}", mqtt_message.topic, why);
                        continue;
                    },
                };
                let subcommand = match SubCommand::from_payload(&mqtt_message.payload) {
                    Some(subcommand) => subcommand,
                    None => {
                        warn!("Unknown payload: {}", mqtt_message.payload);
                        continue;
                    },
                };
                match subcommand {
                    SubCommand::Add { number, carrier, alias } => {
                        if items.get(&number).is_some() {
                            warn!("The item has already been tracked: {} {}", carrier, number);
                            continue;
                        }
                        if let Some(a) = &alias {
                            match &aliases.get(a) {
                                Some(k) => {
                                    warn!("The alias has already been used as '{}': {}", k, a);
                                },
                                None => {
                                    aliases.insert(a.clone(), number.clone());
                                },
                            }
                        }
                        let number_clone = number.clone();
                        let tx = response_tx.clone();
                        let topic = Topic::Response {
                            service: device_id.clone(),
                            session_id: session_id.clone(),
                            is_last: TopicBool::False,
                        }.to_string();
                        let item_handle = tokio::spawn(async move {loop {
                            let mut response = match carrier {
                                Carrier::Jp => fetch_delivery_status_jp(&number_clone).await,
                                Carrier::Yamato => fetch_delivery_status_yamato(&number_clone).await,
                                Carrier::Sagawa => fetch_delivery_status_sagawa(&number_clone).await,
                            };
                            if let Err(why) = response {
                                    warn!("Failed to fetch the tracking data: {}", why);
                                    tokio::time::sleep(Duration::from_secs(retry_interval_seconds)).await;
                                    continue;
                            }
                            let alias_clone = alias.clone();
                            response = response.map(|mut res| {
                                res.alias = alias_clone;
                                res
                            });
                            let payload = serde_json::to_string(&SubCommandResponse::Add(response)).unwrap();
                            let message = MQTTMessage {
                                topic: topic.clone(),
                                payload: payload.clone()
                            };
                            if let Err(why) = tx.send(message).await {
                                warn!("Failed to send tracking data: {}", why);
                            } else {
                                info!("Send tracking data to publisher: {:?}", payload);
                            }
                            tokio::time::sleep(Duration::from_secs(tracking_interval_seconds)).await;
                        }});
                        
                        if items.insert(number.clone(), item_handle).is_some() {
                            error!("Unexpected item: {}", number);
                        }
                    },
                    SubCommand::Delete { number } => {
                        let key = match aliases.get(&number) {
                            Some(n) => n.clone(),
                            None => number.clone(),
                        };
                        let result = match items.get(&key) {
                            Some(item_handle) => {
                                item_handle.abort();
                                let removed_key = aliases.iter().find(|(_, v)| **v == key).map(|(k, _)| k.clone());
                                if let Some(k) = removed_key {
                                    aliases.remove(&k);
                                }
                                if items.remove(&key).is_none() {
                                    warn!("Failed to remove the item: {}", key);
                                    SubCommandResponse::Delete(
                                        Ok(format!("Maybe delete {}.", number))
                                    )
                                } else {
                                    info!("Removed the item: {}", key);
                                    SubCommandResponse::Delete(
                                        Ok(format!("Deleted {}.", number))
                                    )
                                }
                            },
                            None => {
                                warn!("Unknown the removed item: {}", key);
                                SubCommandResponse::Delete(
                                    Ok(format!("No the key {}.", number))
                                )
                            }
                        };
                        let payload = serde_json::to_string(&result).unwrap();
                        let topic = Topic::Response {
                            service: device_id.clone(),
                            session_id: session_id.clone(),
                            is_last: TopicBool::True,
                        }.to_string();
                        let tx = response_tx.clone();
                        if let Err(why) = tx.send(MQTTMessage {topic, payload: payload.clone()}).await {
                            warn!("Failed to send tracking response: {}", why);
                        } else {
                            info!("Send tracking response to publisher: {:?}", payload);
                        }
                    },
                    SubCommand::List { number } => {
                        let payload = match number {
                            Some(n) => {
                                let mut response = HashMap::new();
                                let key = match aliases.get(&n) {
                                    Some(a) => {
                                        a.as_str()
                                    },
                                    None => n.as_str(),
                                };
                                if items.contains_key(key) {
                                    if key == n {
                                        // No alias
                                        response.insert(key.to_string(), None);
                                    }
                                    else {
                                        response.insert(key.to_string(), Some(n.to_string()));
                                    }
                                }
                                let subcommand_response = if response.len() == 0 {
                                    Err(format!("No item: {}", n))
                                } else {
                                    Ok(response)
                                };
                                serde_json::to_string(
                                    &SubCommandResponse::List(subcommand_response)
                                ).unwrap()
                            },
                            None => {
                                let mut response = HashMap::new();
                                for n in items.keys() {
                                    response.insert(n.to_string(), None);
                                }
                                for (k, v) in &aliases {
                                    response.insert(v.to_string(), Some(k.to_string()));
                                }
                                serde_json::to_string(
                                    &SubCommandResponse::List(Ok(response))
                                ).unwrap()
                            }
                        };
                        let topic = Topic::Response {
                            service: device_id.clone(),
                            session_id: session_id,
                            is_last: TopicBool::True,
                        }.to_string();
                        let tx = response_tx.clone();
                        if let Err(why) = tx.send(MQTTMessage { topic, payload: payload.clone() }).await {
                            warn!("Failed to send tracking list: {}", why);
                        } else {
                            info!("Send tracking list to publisher: {:?}", payload);
                        }
                    },
                }
            }
        }) => {
            info!("Command Handler exited.");
        },
    }
}

#[test]
fn test_infer_year() {
    const DATETIME_FORMAT: &str = "%m月%d日 %H:%M";
    
    let reference = DateTime::parse_from_rfc3339("2025-10-02T00:00:00+09:00").unwrap().with_timezone(&Local);
    assert_eq!(
        infer_year("10月1日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-10-01T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("10月3日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-10-03T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("11月1日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-11-01T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("11月2日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2024-11-02T01:23:00+09:00").unwrap())
    );
    
    let reference = DateTime::parse_from_rfc3339("2025-12-02T00:00:00+09:00").unwrap().with_timezone(&Local);
    assert_eq!(
        infer_year("12月1日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-12-01T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("12月3日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-12-03T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("1月1日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2026-01-01T01:23:00+09:00").unwrap())
    );
    assert_eq!(
        infer_year("1月2日 01:23", DATETIME_FORMAT, reference.clone()),
        Ok(DateTime::parse_from_rfc3339("2025-01-02T01:23:00+09:00").unwrap())
    );
}
