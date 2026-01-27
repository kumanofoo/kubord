use log::{error, info, warn};
use std::env;
use std::collections::HashMap;
use kubord::book::BookDb;
use kubord::{GenericError, progname};
use kubord::mqtt::{self, connect_broker, MQTTMessage, Publisher, Subscriber, TopicBool};
use tokio::sync::mpsc::{Receiver, Sender};


/// Handles incoming MQTT messages, searches for books, and sends the results.
///
/// This asynchronous function continuously receives MQTT messages, uses the payload
/// to search for books in the configured libraries, and then sends the original
/// message along with the `BookDb` instance to a publisher for further processing.
///
/// # Arguments
///
/// * `mqtt_rx`: A `Receiver` for receiving `MQTTMessage` instances from an MQTT subscriber.
/// * `appkey`: A string slice representing the application key for accessing the Calil API.
/// * `libraries`: A `HashMap` containing library identifiers and their corresponding names.
/// * `book_tx`: A `Sender` for sending tuples of `MQTTMessage` and `BookDb` to a publisher.
async fn book_handle(
    mut mqtt_rx: Receiver<MQTTMessage>,
    appkey: &str,
    libraries: &HashMap<String, String>,
    book_tx: Sender<(MQTTMessage, BookDb)>) {
    loop {
        match mqtt_rx.recv().await {
            Some(msg) => {
                info!("Received MQTT message: topic={}, payload={}", msg.topic, msg.payload);
                let mut bookdb = BookDb::new(&appkey, libraries);
                match bookdb.search(&msg.payload).await {
                    Ok(_) => {
                        println!("{}", bookdb.json());
                        book_tx.send((msg, bookdb)).await.expect("Failed to send bookdb");
                    },
                    Err(e) => {
                        warn!("Failed to search book: {}", e);
                    }
                }
            },
            None => {
                warn!("No more messages to receive.");
                break;
            }
        }
    }
}

/// The main function for the librarian application.
///
/// This asynchronous function initializes the application, connects to the MQTT broker,
/// sets up the message handling channels, and spawns tasks for subscribing to MQTT topics,
/// handling book searches, and publishing search results.
#[tokio::main]
async fn main() -> Result<(), GenericError> {
    env_logger::init();
    //env_logger::init_from_env(env_logger::Env::new().default_filter_or("librarian=info"));

    let appkey = match env::var("CALIL_APPKEY") {
        Ok(key) => key,
        Err(_) => {
            error!("Expected a CALIL_APPKEY in the environment.");
            std::process::exit(1);
        }
    };

    // Configration
    let config = match kubord::load_config() {
        Ok(config) =>  config,
        Err(why) => {
            error!("Configration file error: {}", why);
            std::process::exit(1);
        }
    };
    let (libraries, service) = match config.book {
        Some(book) => {
            let service = match &book.service {
                Some(service) => service.clone(),
                None => progname(),
            };
            (book.libraries.clone(), service)
        },
        None => {
            error!("'Book' key not found in config.");
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
    let qos = match mqtt_config.qos {
        Some(q) => q,
        None => 0, // Default QoS to 0 if not specified
    };

    // MQTT topic for librarian.
    let request_topic = vec![
        mqtt::Topic::Command {
            //device_id: "library".to_string(),
            service,
            session_id: mqtt::Topic::ANY.to_string(),
        }.to_string()
    ];
    info!("MQTT subscribes topics: {:?}", request_topic);
    
    let mqtt_client = connect_broker(&mqtt_config).await?;
    info!("Connected to MQTT broker: {}", mqtt_config.broker);

    // Setup subscriber and publisher.
    let mut subscriber = Subscriber::new_from_client(mqtt_client.clone(), qos);
    let publisher = Publisher::new_from_client(mqtt_client.clone());

    // Setup Channel between MQTT and Book searching.
    // [subscriber] -> mqtt_tx | mqtt_rx -> [book_hande] -> book_tx | book_rx -> [publisher]
    let (mqtt_tx, mqtt_rx) = tokio::sync::mpsc::channel::<MQTTMessage>(100);
    let (book_tx, mut book_rx) = tokio::sync::mpsc::channel::<(MQTTMessage, BookDb)>(100);

    // Start the subscriber
    info!("Librarian started.");
    tokio::select! {
        _ = subscriber.run(request_topic, mqtt_tx) => {
            info!("Subscriber task exited.");
        },
        _ = book_handle(mqtt_rx, &appkey, &libraries, book_tx) => {
            info!("Book handle task exited.");
        },
        _ = tokio::spawn(async move {
            info!("Waiting for MQTT messages...");
            while let Some((msg, bookdb)) = book_rx.recv().await {
                let topic: mqtt::Topic = match msg.topic.parse() {
                    Ok(t) => t,
                    Err(why) => {
                        warn!("Failed to parse the topic: {}: {:?}", msg.topic, why);
                        continue;
                    }
                };
                let (service, session_id) = match topic {
                    mqtt::Topic::Command {service, session_id} => (service, session_id),
                    _ => {
                        warn!("Unknown topic: {}", msg.topic);
                        continue;
                    }
                };
                let response_topic = mqtt::Topic::Response { service, session_id, is_last: TopicBool::True }.to_string();
                info!("Publishing response to topic: {}", response_topic);
                let response = bookdb.json();
                publisher.publish(&response_topic, &response).await;
            }
        }) => {
            info!("MQTT message handling task exited.");
        },
    }
    
    Ok(())
}

/// Tests the BookDb functionality.
///
/// This test function retrieves the Calil application key from the environment,
/// creates a `BookDb` instance with some sample libraries, searches for a book,
/// and asserts that the search returns a positive number of books.
#[tokio::test]
async fn test_bookdb() {
    let appkey = env::var("CALIL_APPKEY").unwrap();
    let mut libraries = HashMap::new();
    libraries.insert("Tokyo_NDL".to_string(), "Tokyo National Library".to_string());
    libraries.insert("Tokyo_Pref".to_string(), "Tokyo Metropolitan Library".to_string());
    let mut bookdb = BookDb::new(&appkey, &libraries);
    
    let title = "Effective Python";
    let book_num = bookdb.search(title).await.unwrap();
    assert!(book_num > 0);
    bookdb.display();
}
