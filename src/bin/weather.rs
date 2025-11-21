use log::{info, warn, error};
use clap::Parser;

use jma::forecast::JmaForecast;
use jma::amedas::{
    station_information, Amedas, AmedasData, AmedasStation
};
use jma::forecast_area::ForecastArea;
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;
use serde::{Deserialize, Serialize};
use kubord::Config;
use kubord::mqtt::{Publisher, Topic};

async fn temperature_publisher(token: CancellationToken, config: std::sync::Arc<Config>) {
    let (temperature_station, temperature_interval_hours) = match &config.weather {
        Some(weather) => {
            let temperature_station = match &weather.temperature_station {
                Some(station) => station.clone(),
                None => weather.amedas_station.clone(),
            };
            (temperature_station, weather.temperature_interval_hours)
        },
        None => return,
    };
    let office = match ForecastArea::new().await {
        Ok(areas) => {
            match areas.get_office_by_amedas(&temperature_station) {
                Some(office) => office,
                None => {
                    error!("No JMA office code from station code {}.", temperature_station);
                    return;
                }
            }
        },
        Err(why) => {
            error!("Failed to search JMA offices: {}", why);
            return;
        }
    };
    let mqtt_config = match &config.mqtt {
        Some(mqtt) => mqtt,
        None => return,
    };
    
    let publisher = match Publisher::new(&mqtt_config).await {
        Ok(p) => p,
        Err(why) => {
            error!("Failed to create Temperature publisher: {}", why);
            token.cancel();
            return;
        },
    };
    
    let topic = Topic::Monitor {
        device_id: "temperature".to_string(),
    }.to_string();

    let mut interval = interval(Duration::from_secs(temperature_interval_hours*60*60));
    info!("Temperature publisher start ...");
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let forecast = match JmaForecast::new(&office).await {
                    Ok(f) => f,
                    Err(why) => {
                        warn!("Failed to fetch forecast: {}", why);
                        continue;
                    },
                };
                let peak = match forecast.temperature_forecast(&temperature_station) {
                    Some(p) => p,
                    None => {
                        warn!("Temperature is not found");
                        continue;
                    },
                };
                let payload_str = match serde_json::to_string(&peak) {
                    Ok(p) => p,
                    Err(why) => {
                        warn!("Failed to convert into JSON: {}", why);
                        continue;
                    },
                };
                publisher.publish(&topic, &payload_str).await;
                info!("publish: {} - {}", topic, payload_str);
            }
            _ = token.cancelled() => {
                info!("Temperature publisher received a cancel token ...");
                break;
            }
        }
    }
}

type Kanji = String;
type English = String;
type Code = String;

#[derive(Debug, Deserialize, Serialize, Clone)]
struct AmedasPayload {
    station: (Kanji, English, Code),
    latest_time: String,
    data: AmedasData,
}

impl AmedasPayload {
    fn new(amedas: &Amedas, amedas_station: &str, station_info: &AmedasStation) -> Self {
        let latest = AmedasData::from(&amedas.get_latest_data().unwrap());
        AmedasPayload {
            station: (
                station_info.kanji_name.to_string(),
                station_info.english_name.to_string(),
                amedas_station.to_string(),
            ),
            latest_time: amedas.latest_time.to_string(),
            data: latest.clone(),
        }
    }
    
    fn json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}


async fn amedas_publisher(token: CancellationToken, config: std::sync::Arc<Config>)  {
    let (amedas_station, interval_minutes) = match &config.weather {
        Some(weather) => (weather.amedas_station.clone(), weather.amedas_interval_minutes),
        None => return,
    };

    let mqtt_config = match &config.mqtt {
        Some(mqtt) => mqtt,
        None => return,
    };
    
    let publisher = match Publisher::new(&mqtt_config).await {
        Ok(p) => p,
        Err(why) => {
            error!("Failed to create AMeDAS publisher: {}", why);
            token.cancel();
            return;
        },
    };
    let station_info = match station_information(&amedas_station).await {
        Ok(info) => info,
        Err(why) => {
            error!("Failed to fetch AMeDAS station information: {}", why);
            token.cancel();
            return;
        }
    };
    
    let topic = Topic::Sensor {
        location: format!("amedas"),
        device_id: amedas_station.to_string(),
    }.to_string();
    
    let mut amedas = match Amedas::new(&amedas_station).await {
        Ok(amedas) => amedas,
        Err(why) => {
            error!("Failed to fetch AMeDAS data: {}", why);
            token.cancel();
            return;
        }
    };
    
    let payload_str = AmedasPayload::new(&amedas, &amedas_station, &station_info).json();
    publisher.publish(&topic, &payload_str).await;
    info!("publish: {} - {}", topic, payload_str);

    let mut interval = interval(Duration::from_secs(interval_minutes*60));
    info!("Amedas publisher start ...");
    loop {
        tokio::select! {
            _ = interval.tick() => {
                match amedas.update().await {
                    Ok(is_new) => {
                        if is_new {
                            let payload_str = AmedasPayload::new(&amedas, &amedas_station, &station_info).json();
                            publisher.publish(&topic, &payload_str).await;
                            info!("publish: {} - {}", topic, payload_str);
                        }
                        else {
                            info!("No update data");
                        }
                    },
                    Err(why) => {
                        warn!("Failed to update AMeDAS data: {}", why);
                        continue;
                    },
                }
            }
            _ = token.cancelled() => {
                info!("Amedas publisher received a cancel token ...");
                break;
            }
        }
    }
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
    let config = std::sync::Arc::new(
        match kubord::load_config_with_filename(&args.config_path) {
            Ok(config) => config,
            Err(why) => {
                error!("Configuration file error: {}", why);
                std::process::exit(1);
            }
        }
    );
    if config.mqtt.is_none() {
        error!("No MQTT configuration.");
        std::process::exit(1);
    }
    if config.weather.is_none() {
        error!("No Weather configuration.");
        std::process::exit(1);
    }
    
    let token = CancellationToken::new();
    let config_amedas = std::sync::Arc::clone(&config);
    let task_amedas = tokio::spawn(amedas_publisher(token.clone(), config_amedas));
    let config_temperature = std::sync::Arc::clone(&config);
    let task_temperature = tokio::spawn(temperature_publisher(token.clone(), config_temperature));
    let _ = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Detected Ctrl-C. Shutdown all threads ...");
        token.cancel();
    });

    let _ = tokio::join!(task_amedas, task_temperature);
}
