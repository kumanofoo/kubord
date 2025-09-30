use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use csv::{ReaderBuilder, WriterBuilder};
use kubord::{mqtt, PingConfig, progname};
use log::{error, info, warn};
use regex::Regex;
use std::process::Command;
use std::{fs, io, path::Path, time::Duration};
use serde::{Deserialize, Serialize};
use std::fmt;


const PING_COUNT: usize = 10;

#[derive(Debug)]
enum RttMonitorError {
    NotEnoughHistoricalData,
    Io(io::Error),
}

impl std::fmt::Display for RttMonitorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEnoughHistoricalData => write!(f, "Not enough historical data to detect anomalies"),
            Self::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for RttMonitorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotEnoughHistoricalData => None,
            Self::Io(e) => Some(e),
        }
    }
}

impl From<io::Error> for RttMonitorError {
    fn from(err: io::Error) -> Self {
        RttMonitorError::Io(err)
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
struct RttRecord {
    timestamp: i64,     // Unix timestamp in milliseconds
    host: String,       // Hostname
    rtt_ms: f64,        // Round Trip Time in milliseconds
    loss_percent: f64,  // Packet loss in percentage
}

impl fmt::Display for RttRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Host: {}, RTT: {}, Loss: {}, Timestamp: {}",
            self.host, self.rtt_ms, self.loss_percent, self.timestamp
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RttStatistiscs {
    mean_rtt_ms: f64,
    sd_rtt_ms: f64,
}

impl fmt::Display for RttStatistiscs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RTT Statistics: mean = {:.2} ms, sd = {:.2} ms",
            self.mean_rtt_ms, self.sd_rtt_ms
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Report {
    rtt_record: Option<RttRecord>,
    rtt_upper_bound: Option<f64>, // Optional upper bound for RTT
    anomaly: mqtt::AnomalyKind,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[RTT REPORT]")?;
        if let Some(rtt_record) = &self.rtt_record {
            writeln!(f, "- Record: {}", rtt_record)?;
        } else {
            writeln!(f, "- Record: None")?;
        }
        write!(f, "- Anomaly: {}", self.anomaly)
    }
}

fn append_rtt_record(record: &RttRecord, data_file: &str) -> io::Result<()> {
    let file_exists = Path::new(data_file).exists();
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(data_file)?;

    let mut wtr = WriterBuilder::new()
        .has_headers(!file_exists)
        .from_writer(file);
    wtr.serialize(record)?;
    wtr.flush()?;
    Ok(())
}

fn load_and_prune_rtt_data(
    current_time: DateTime<Utc>,
    data_file: &str,
    history_duration_hours: u64
) -> io::Result<Vec<RttRecord>> {
    let mut records = Vec::new();
    if !Path::new(data_file).exists() {
        return Ok(records);
    }

    let file = fs::File::open(&data_file)?;
    let mut rdr = ReaderBuilder::new().from_reader(file);

    for result in rdr.deserialize() {
        let record: RttRecord = result?;
        let record_time = Utc.timestamp_opt(record.timestamp, 0).unwrap();
        if current_time - record_time <= ChronoDuration::hours(history_duration_hours as i64) {
            records.push(record);
        }
    }

    // Prune old records and rewrite the file
    let mut wtr = WriterBuilder::new().from_path(&data_file)?;
    for record in &records {
        wtr.serialize(record)?;
    }
    wtr.flush()?;

    Ok(records)
}

fn rtt_statistics(host: &str, config: &PingConfig) -> Result<RttStatistiscs, RttMonitorError> {
    let historical_data = match load_and_prune_rtt_data(Utc::now(), &config.data_filename(host), config.history_duration_hours) {
        Ok(data) => data,
        Err(e) => {
            return Err(RttMonitorError::Io(e));
        }
    };
    if historical_data.len() < 2 {
        return Err(RttMonitorError::NotEnoughHistoricalData);
    }
    let mean_rtt_ms =
        historical_data.iter().map(|r| r.rtt_ms).sum::<f64>() / historical_data.len() as f64;
    let sd_rtt_ms = (historical_data
        .iter()
        .map(|r| r.rtt_ms)
        .map(|rtt| (rtt - mean_rtt_ms).powi(2))
        .sum::<f64>()
        / historical_data.len() as f64)
        .sqrt();
    Ok(RttStatistiscs { mean_rtt_ms, sd_rtt_ms })
}

#[test]
fn test_append_and_load_and_statistics() {
    // 1. Setup
    use std::path::PathBuf;
    let tempdir = tempfile::tempdir().unwrap();
    let mut data_file_path = PathBuf::from(tempdir.path());
    data_file_path.push("test_rtt_data");
    let data_file_prefix = data_file_path.to_str().unwrap();
    let host = "www.example.com";
    let config = PingConfig {
        data_file_prefix: data_file_prefix.to_string(),
        history_duration_hours: 24,
        ping_interval_minutes: 6,
        anomaly_threshold_factor: 2.0,
        hosts: vec![host.to_string()],
        device_id: None,
    };
    let data_file = config.data_filename(host);

    let mut records_to_write = Vec::new();
    let host = config.hosts[0].clone();
    const NUM_RECORDS: i64 = 48*10;   // 48 hours worth of records
    const RTT_FACTOR: f64 = 1.5; // Simulated RTT factor
    let base_timestamp = Utc::now().timestamp() - 2*24*60*60; // Base timestamp for 2 days ago
    let interval_sec= (config.ping_interval_minutes * 60) as i64; // Convert minutes to seconds

    for i in 0..NUM_RECORDS {
        let record = RttRecord {
            timestamp: base_timestamp + i * interval_sec, // Simulated timestamp in seconds
            host: host.clone(),
            rtt_ms: (i as f64) * RTT_FACTOR, // Simulated RTT values
            loss_percent: (i % 10) as f64, // Simulated packet loss values
        };
        records_to_write.push(record);
    }

    // 2. Write records to the file
    for record in &records_to_write {
        append_rtt_record(record, &data_file).unwrap();
    }

    // 3. Verify that the records were written correctly
    let file = fs::File::open(&data_file).unwrap();
    let mut rdr = ReaderBuilder::new()
        .from_reader(file);
    let mut loaded_records = Vec::new();
    for result in rdr.deserialize() {
        let record: RttRecord = result.unwrap();
        loaded_records.push(record);
    }
    assert_eq!(loaded_records.len(), records_to_write.len(), "Number of records should match");
    assert_eq!(loaded_records, records_to_write, "Record contents should match");

    // 4. Load and prune historical data
    let current_time = Utc::now();
    let pruned_records = load_and_prune_rtt_data(current_time, &data_file, config.history_duration_hours).unwrap();
    assert_eq!(pruned_records.len(), 60*24/6-1, "Pruned records should be less than or equal to original records");

    // 5. Calculate statistics
    let statistics = rtt_statistics(&host, &config).unwrap();
    assert_eq!(statistics.mean_rtt_ms,  540.0, "Mean RTT should be 540.0 ms");
    assert!(statistics.sd_rtt_ms > 103.4 && statistics.sd_rtt_ms < 103.5, "Standard deviation should be around 103.489 ms");
}

/// ```bash
/// $ ping -4 -c 1 www.google.com
/// PING www.google.com (142.250.199.100) 56(84) bytes of data.
/// 64 bytes from nrt13s52-in-f4.1e100.net (142.250.199.100): icmp_seq=1 ttl=114 time=17.8 ms
///
/// --- www.google.com ping statistics ---
/// 1 packets transmitted, 1 received, 0% packet loss, time 0ms
/// rtt min/avg/max/mdev = 17.801/17.801/17.801/0.000 ms
/// ```
fn rtt(host: &str, count: usize) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    // ping
    let output = Command::new("ping")
        .arg("-4")
        .arg("-c")
        .arg(count.to_string())
        .arg(host)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Ping command failed: {}", stderr).into());
    }

    // Extract RTT from stdout
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Try to extract RTT from the line of statstics.
    let re_avg = Regex::new(r"= \d+\.\d+/(\d+\.\d+)/\d+\.\d+/\d+\.\d+ ms")?;
    let captures = match re_avg.captures(&stdout) {
        Some(c) => c,
        None => {
            return Err("Could not extract RTT statistics from ping output."
                .to_string()
                .into())
        }
    };
    let rtt_avg = match captures.get(1) {
        Some(avg) => avg.as_str().parse::<f64>()?,
        None => return Err("RTT value not found in ping statistics.".to_string().into()),
    };
    let re_loss = Regex::new(r", (\d+)% packet loss,")?;
    let captures = match re_loss.captures(&stdout) {
        Some(c) => c,
        None => {
            return Err("Could not extract packet loss from ping output"
                .to_string()
                .into())
        }
    };
    let loss = match captures.get(1) {
        Some(loss) => loss.as_str().parse::<f64>()?,
        None => {
            return Err("Packet loss value not found in ping statistics."
                .to_string()
                .into())
        }
    };
    Ok((rtt_avg, loss))
}


fn create_report(host: &str, config: &PingConfig) -> Report {
    let current_time = Utc::now();

    let mut report = Report {
        rtt_record: None,
        rtt_upper_bound: None,
        anomaly: mqtt::AnomalyKind::None,
    };

    let (rtt_ms, loss_percent) = match rtt(&host, PING_COUNT) {
        Ok(res) => res,
        Err(e) => {
            report.anomaly = mqtt::AnomalyKind::FailToPing(e.to_string());
            return report;
        },
    };

    let statistics = rtt_statistics(host, config);
    let rtt_record = RttRecord {
        timestamp: current_time.timestamp(),
        host: host.to_string(),
        rtt_ms: rtt_ms,
        loss_percent: loss_percent,
    };
    if let Err(e) = append_rtt_record(&rtt_record, &config.data_filename(host)) {
        warn!("{}", e);
    }
    report.rtt_record = Some(rtt_record);

    let anomaly_threshold = match statistics {
        Ok(stat) => {
            info!("RTT statistics for {}: mean = {:.2} ms, sd = {:.2} ms", host, stat.mean_rtt_ms, stat.sd_rtt_ms);
            // Calculate the anomaly threshold
            stat.mean_rtt_ms + config.anomaly_threshold_factor * stat.sd_rtt_ms
        },
        Err(e) => {
            match e {
                RttMonitorError::NotEnoughHistoricalData => {
                    info!("Not enough historical data to calculate threshold for {}", host);
                    return report;
                }
                RttMonitorError::Io(e) => {
                    warn!("Failed to load historical data for {}: {}", host, e);
                    report.anomaly = mqtt::AnomalyKind::RttData(e.to_string());
                    return report;
                }
            }
        }
    };
    report.rtt_upper_bound = Some(anomaly_threshold);

    if rtt_ms > anomaly_threshold {
        report.anomaly = mqtt::AnomalyKind::AnomalyRtt(rtt_ms, anomaly_threshold);
        info!("Anomaly detected for {}: RTT {} ms exceeds threshold {} ms", host, rtt_ms, anomaly_threshold);
        return report;
    }
    if loss_percent > 0.0 {
        report.anomaly = mqtt::AnomalyKind::PacketLoss(loss_percent);
        info!("Anomaly detected for {}: Packet loss {}%", host, loss_percent);
        return report;
    }

    // No anomalies detected
    info!("No anomalies detected for {}: RTT {} ms, Packet loss {}%", host, rtt_ms, loss_percent);
    report
}

#[test]
fn test_create_report() {
    // 1. RTT
    let localhost = "localhost";
    let invalidhost = "dns.invalid";
    let (localhost_rtt_ms, localhost_loss_percent) = rtt(localhost, PING_COUNT).unwrap();
    assert!(localhost_rtt_ms > 0.0, "RTT should be greater than 0");
    assert_eq!(localhost_loss_percent, 0.0, "Packet loss should be 0% for localhost");
    let invalidhost_result = rtt(invalidhost, PING_COUNT);
    assert!(invalidhost_result.is_err(), "Ping to invalid host should fail");

    // 2. Create a temporary config
    use std::path::PathBuf;
    let tempdir = tempfile::tempdir().unwrap();
    let mut data_file_path = PathBuf::from(tempdir.path());
    data_file_path.push("test_rtt_data");
    let data_file_prefix = data_file_path.to_str().unwrap();
    let config = PingConfig {
        data_file_prefix: data_file_prefix.to_string(),
        history_duration_hours: 24,
        ping_interval_minutes: 6,
        anomaly_threshold_factor: 2.0,
        hosts: vec![localhost.to_string(), invalidhost.to_string()],
        device_id: None,
    };
    let data_file = config.data_filename(localhost);

    // 3. No historical data
    let report = create_report(localhost, &config);
    assert!(report.rtt_record.is_some(), "Report should contain RTT record");
    let rtt_record = report.rtt_record.as_ref().unwrap();
    assert_eq!(rtt_record.host, localhost, "RTT record host should match");
    assert!(rtt_record.rtt_ms > 0.0, "RTT should be greater than 0");
    assert_eq!(report.anomaly, mqtt::AnomalyKind::None, "No anomaly should be detected for localhost");

    // 4. Invalid host
    let invalid_report = create_report(invalidhost, &config);
    assert!(invalid_report.rtt_record.is_none(), "Report for invalid host should not contain RTT record");
    match invalid_report.anomaly {
        mqtt::AnomalyKind::FailToPing(_) => {}, // Expected
        _ => panic!("Expected FailToPing anomaly for invalid host"),
    }
    info!("Anomaly detected for invalid host: {:?}", invalid_report.anomaly);
    assert!(invalid_report.rtt_upper_bound.is_none(), "RTT upper bound should not be set for invalid host");
    assert!(invalid_report.rtt_record.is_none(), "RTT record should not be present for invalid host");

    //  avg ,  mdev, rasio
    // 0.026, 0.003, 0.115
    // 0.087, 0.002, 0.023
    // 0.286, 0.025, 0.087
    // 0.041, 0.006, 0.146
    use rand::thread_rng;
    use rand_distr::{Distribution, Normal};
    let mut rng = thread_rng();
    let num_samples = config.history_duration_hours * 60 / config.ping_interval_minutes;

    // 4. Normal report
    let std_dev = localhost_rtt_ms * 0.15; // Standard deviation for simulated RTT
    let normal = Normal::new(localhost_rtt_ms, std_dev).unwrap(); // Simulated RTT around the measured value
    let base_timestamp = Utc::now().timestamp() - (config.history_duration_hours as i64 * 60 * 60);
    for i in 0..num_samples {
        // Simulate some RTT data
        let rtt_record = RttRecord {
            timestamp: base_timestamp + (i as i64 * config.ping_interval_minutes as i64 * 60),
            host: localhost.to_string(),
            rtt_ms: normal.sample(&mut rng),
            loss_percent: 0.0, // No packet loss for simulated data
        };
        append_rtt_record(&rtt_record, &data_file).unwrap();
    }

    let report = create_report(localhost, &config);
    assert!(report.rtt_record.is_some(), "Report should contain RTT record");
    let rtt_record = report.rtt_record.as_ref().unwrap();
    assert_eq!(rtt_record.host, localhost, "RTT record host should match");
    assert!(rtt_record.rtt_ms > 0.0, "RTT should be greater than 0");
    assert_eq!(report.anomaly, mqtt::AnomalyKind::None, "No anomaly should be detected for localhost");

    // 5. Anomaly report
    fs::remove_file(&data_file).unwrap(); // Clean up the data file after the test
    let rtt_ms_shifted = localhost_rtt_ms * 0.5; // Simulated RTT that exceeds the threshold
    let std_dev_shifted = rtt_ms_shifted * 0.01; // Standard deviation for simulated RTT
    let normal = Normal::new(rtt_ms_shifted, std_dev_shifted).unwrap(); // Simulated RTT around the measured value
    let base_timestamp = Utc::now().timestamp() - (config.history_duration_hours as i64 * 60 * 60);
    for i in 0..num_samples {
        // Simulate some RTT data
        let rtt_record = RttRecord {
            timestamp: base_timestamp + (i as i64 * config.ping_interval_minutes as i64 * 60),
            host: localhost.to_string(),
            rtt_ms: normal.sample(&mut rng),
            loss_percent: 0.0, // No packet loss for simulated data
        };
        append_rtt_record(&rtt_record, &data_file).unwrap();
    }

    let report = create_report(localhost, &config);
    assert!(report.rtt_record.is_some(), "Report should contain RTT record");
    let rtt_record = report.rtt_record.as_ref().unwrap();
    assert_eq!(rtt_record.host, localhost, "RTT record host should match");
    assert!(rtt_record.rtt_ms > 0.0, "RTT should be greater than 0");
    match report.anomaly {
        mqtt::AnomalyKind::AnomalyRtt(_, _) => {}, // Expected anomaly
        _ => panic!("Expected AnomalyRtt anomaly for localhost"),
    }
}



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //env_logger::init_from_env(env_logger::Env::new().default_filter_or("rtt_monitor=info"));
    env_logger::init();

    let config = match kubord::load_config() {
        Ok(config) => config,
        Err(why) => {
            error!("Configration file not found: {:?}", why);
            std::process::exit(1);
        }
    };
    let ping_config = match config.ping {
        Some(ping) => ping,
        None => {
            error!("'ping' key not found in config.");
            std::process::exit(1);
        }
    };

    let mqtt_config = match config.mqtt {
        Some(mqtt) => mqtt,
        None => {
            error!("'mqtt' key not found in config.");
            std::process::exit(1);
        }
    };

    // MQTT topic for ping.
    let device_id = match &ping_config.device_id {
        Some(device_id) => device_id.clone(),
        None => progname(),
    };
    let ping_topic = mqtt::Topic::Monitor { device_id }.to_string();
    let mqtt_client = mqtt::connect_broker(&mqtt_config).await.unwrap();
    let publisher = mqtt::Publisher::new_from_client(mqtt_client.clone());

    info!(
        "RTT Monitor started. Checking every {} minutes.",
        ping_config.ping_interval_minutes
    );

    loop {
        let current_time = Utc::now();
        info!("[{}] Peforming RTT check...", current_time.to_rfc3339());

        let mut tasks = tokio::task::JoinSet::new();
        for host in &ping_config.hosts {
            info!("ping to {}...", host);
            let ping_config_clone = ping_config.clone();
            let host_clone = host.clone();
            tasks.spawn(async move {
                create_report(&host_clone, &ping_config_clone)
            });
        }

        let mut anomalies = Vec::new();
        while let Some(ref result) = tasks.join_next().await {
            match result {
                Ok(report) => {
                    if report.anomaly != mqtt::AnomalyKind::None {
                        anomalies.push(mqtt::Anomaly {
                            host: report.rtt_record.as_ref().map_or_else(|| "unknown".to_string(), |r| r.host.clone()),
                            timestamp: report.rtt_record.as_ref().map_or(0, |r| r.timestamp),
                            anomaly: report.anomaly.clone(),
                        });
                    }
                }
                Err(e) => {
                    error!("Error creating report: {}", e);
                    continue;
                }
            }
        }

        let regular_report = mqtt::AnomalyReport {
            timestamp: Utc::now().timestamp(),
            anomalies: anomalies.clone(),
        };
        if anomalies.is_empty() {
            info!("No anomalies detected in this round.");
        } else {
            info!("Anomalies detected: {}", serde_json::to_string(&anomalies).unwrap());
        }

        let regular_report_str = serde_json::to_string(&regular_report).unwrap();
        info!("Topic: {}", ping_topic);
        info!("Report: {:?}", regular_report);
        publisher.publish(&ping_topic, &regular_report_str).await;
 
        // Sleep for the specified interval
        tokio::time::sleep(Duration::from_secs(ping_config.ping_interval_minutes * 60)).await;
    }
}
