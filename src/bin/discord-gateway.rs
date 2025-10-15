use log::{info, warn, error};

use serenity::all::{
    CreateInteractionResponse,
    CreateInteractionResponseFollowup,
    CreateInteractionResponseMessage
};
use serenity::async_trait;
use uuid::Uuid;
use std::env;
use serenity::builder::CreateMessage;
use serenity::model::application::{
    Interaction,
    CommandInteraction,
};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::GuildId;
use serenity::prelude::*;
use tokio::sync::mpsc::{channel, Sender};
use tokio::time::{sleep, Duration};
use std::sync::{RwLock, LazyLock};
use std::collections::HashMap;
use kubord::mqtt::{Subscriber, Publisher, MQTTMessage, Topic};
use kubord::{DiscordConfig, GenericError};
use kubord::commands;
use clap::Parser;

static SESSION: LazyLock<RwLock<HashMap<String, (Context, CommandInteraction)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
struct Session;
impl Session {
    fn new(ctx: Context, rply: CommandInteraction) -> Result<String, ()> {
        let session_id = Uuid::new_v4().to_string();
        SESSION.write().unwrap().insert(session_id.clone(), (ctx, rply));
        Ok(session_id)
    }

    fn get(session_id: &str) -> Option<(Context, CommandInteraction)> {
        let mut session = SESSION.write().unwrap();
        session.remove(session_id)
    }

    async fn set_timer(session_id: &str) -> Result<String, ()> {
        if SESSION.read().unwrap().get(session_id).is_none() {
            return Err(());
        }

        let cloned_session_id = session_id.to_string();
        info!("Session timer start: {}", session_id);
        tokio::spawn( async move {
            sleep(Duration::from_secs(120)).await;
            let mut session = SESSION.write().unwrap();
            if let Some((_, cmd)) = session.remove(&cloned_session_id) {
                info!("Session {} timed out. Command: {}",
                      cloned_session_id,
                      cmd.data.name.as_str()
                );
            }
        });
        Ok(session_id.to_string())
    }
}

struct MqttBot {
    token: String,
    config: std::sync::Arc<kubord::Config>,
}

impl MqttBot {
    pub fn new(token: String, config: std::sync::Arc<kubord::Config>) -> Self {
        Self {
            token,
            config,
        }
    }

    async fn run(&self, tx: Sender<MQTTMessage>) {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;
        let discord_config = match &self.config.discord {
            Some(conf) => conf,
            None => {
                error!("No Discord configuration.");
                std::process::exit(1);
            },
        };
        let handler = Handler::new(tx, discord_config.clone());
        let mut client = Client::builder(&self.token, intents)
            .event_handler(handler)
            .await.expect("Error creating client.");
        
        let _ = client.start().await;
    }

    async fn send_folloup(
        ctx: &Context, msg: &CommandInteraction,
        content: CreateInteractionResponseFollowup
    ) -> Result<(), GenericError> {
        msg.create_followup(&ctx.http, content).await?;
        Ok(())
    }
}

struct Handler {
    tx: Sender<MQTTMessage>,
    config: DiscordConfig,
}

impl Handler {
    fn new(tx: Sender<MQTTMessage>, config: DiscordConfig) -> Self {
        Self { tx, config }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if !self.config.channel_id.contains(&msg.channel_id.to_string()) {
            return;
        }

        // Acknowledge
        if let Err(why) = msg.channel_id.say(&ctx.http, "Ok!").await {
            warn!("Sending message: {why:?}");
        }

        // Maybe do something
        sleep(Duration::from_secs(120)).await;

        // Send messages
        match msg.channel_id.send_message(&ctx.http,CreateMessage::new().content("Done!")).await {
            Ok(_) => (),
            Err(err) => warn!("Failed to send: {}", err),
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!("Received command interaction: {command:#?}");
            let service = match self.config.commands.get(&command.data.name) {
                Some(service) => service,
                None => {
                    warn!("No slash command in configuration: {}", command.data.name);
                    return;
                },
            };
            match service.as_str() {
                "librarian" => {
                    let payload = match commands::librarian::handle_command(&ctx, &command).await {
                        Some(request_librarian) => request_librarian,
                        None => return,
                    };
                    let session_id = Session::new(ctx, command).unwrap();
                    let topic = Topic::Command {
                        service: service.to_string(), session_id: session_id.clone()
                    }.to_string();
                    let mqtt_message = MQTTMessage {topic, payload};
                    if Session::set_timer(&session_id).await.is_err() {
                        warn!("Failed to set time: {}", session_id);
                    }
                    self.tx.send(mqtt_message.clone()).await.expect("Failed to send a message to the broker.");
                    info!("Message sent to MQTT: topic={}, payload={}", mqtt_message.topic, mqtt_message.payload);
                }
                _ => {
                    warn!("Not implemented slash command.");
                    let unknown = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("not implelented :(")
                    );
                    if let Err(why) = command.create_response(&ctx.http, unknown).await {
                        warn!("Failed to send unknown: {:?}", why);
                    }
                }
            }
        }
    }
    
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        // register slash commands
        let guild = env::var("GUILD_ID")
            .expect("Expected a token in the environment.")
            .parse::<u64>()
            .expect("GUILD_ID must be an integer");
        let guild_id = GuildId::new(guild);
        let mut create_commands = Vec::new();
        for  (name, service) in &self.config.commands {
            match service.as_str() {
                "librarian" => create_commands.push(
                    commands::librarian::register(&name)
                ),
                _ => (),
            }
        }
        match guild_id.set_commands(&ctx.http, create_commands).await {
            Ok(cmd) => {
                for c in cmd {
                    info!("Add command: {}", c.name);
                }
            },
            Err(why) => {
                error!("Failed to register slash commands: {}", why);
            }
        }
    }
}

#[derive(Parser)]
#[command(version = env!("CARGO_PKG_VERSION"), about, long_about = None)]
struct Args {
    #[arg(short = 'c', long = "config", default_value = "./config.toml")]
    config_path: String,
    #[arg(short = 'l', long = "list")]
    list: bool,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    
    let args = Args::parse();
    let config = std::sync::Arc::new(
        match kubord::load_config_with_filename(&args.config_path) {
            Ok(config) => config,
            Err(why) => {
                error!("Configration file error: {}", why);
                std::process::exit(1);
            }
        }
    );

    if args.list {
        if let Some(discord) = &config.discord {
            println!("Discord Slash commands:");
            for key in discord.commands.keys() {
                println!("  /{}", key);
            }
        } else {
            println!("No Discord configuration found.");
        }
        return;
    }

    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment.");
    env::var("GUILD_ID")
        .expect("Expected a token in the environment.")
        .parse::<u64>()
        .expect("GUILD_ID must be an integer");

    let config_mqtt_bot = std::sync::Arc::clone(&config);
    let mqtt_bot = MqttBot::new(token.clone(), config_mqtt_bot);
    let mqtt_config = config.mqtt.as_ref().unwrap();
    let commands = match config.discord.as_ref() {
        Some(conf) => &conf.commands,
        None => {
            error!("No Commands in Discord configuration.");
            std::process::exit(1);
        }
    };
    let mut topics_to_subscribe = Vec::new();
    for service in commands.values() {
        topics_to_subscribe.push(
            Topic::Response {
                service: service.to_string(),
                session_id: Topic::ANY.to_string(),
            }.to_string()
        );
    }
    info!("Topics to subscribe: {:?}", topics_to_subscribe);
    
    let client = kubord::mqtt::connect_broker(&mqtt_config)
        .await
        .expect("Failed to connect to MQTT broker");
    let mqtt_pub = Publisher::new_from_client(client.clone());
    let mut mqtt_sub = Subscriber::new_from_client(
        client,
        mqtt_config.qos_or_default(),
    );

    // Discord => Discord Message Handler (mqtt_bot) => mqtt_tx:channel:mqtt_rx => MQTT Publisher  
    let (mqtt_tx, mut mqtt_rx) = channel::<MQTTMessage>(128);
    // MQTT Subscriber => discord_tx:channel:discord_rx => MQTT Message Handler => Discord API
    let (discord_tx, mut discord_rx) = channel::<MQTTMessage>(128);
    // Wait messages from the channel and publish it.
    let _mqtt_publisher = tokio::spawn(async move {
        while let Some(msg) = mqtt_rx.recv().await {
            mqtt_pub.publish(&msg.topic, &msg.payload).await;
        }
    });

    let _mqtt_message_handler = tokio::spawn(async move {
        while let Some(mqtt_msg) = discord_rx.recv().await {
            info!("Received from Discord: topic={}, payload={}", mqtt_msg.topic, mqtt_msg.payload);
            let topic = match mqtt_msg.topic.parse::<Topic>() {
                Ok(topic) => topic,
                Err(why) => {
                    warn!("Unexpected MQTT topic '{}': {:?}", mqtt_msg.topic, why);
                    continue;
                }
            };
            match topic {
                Topic::Response { service, session_id } => {
                    let (ctx, cmd) = match Session::get(&session_id) {
                        Some(session) => session,
                        None => {
                            warn!("Session not found for topic: {}", mqtt_msg.topic);
                            continue;
                        }
                    };
                    match service.as_str() {
                        "librarian" => {
                            match commands::librarian::handle_response(&mqtt_msg.payload) {
                                Some(content) => {
                                    MqttBot::send_folloup(&ctx, &cmd, content)
                                        .await
                                        .unwrap_or_else(|err| warn!("Failed to send followup: {}", err));
                                },
                                None => {
                                    warn!("Reply to '{}' command.", cmd.data.name.as_str());
                                }
                            }
                        },
                        _ => {
                            warn!("unknown service: {}", service);
                            continue;
                        }
                    }
                },
                _ => continue,
            };
            
        }
    });
    let _mqtt_subscriber= tokio::spawn(async move {mqtt_sub.run(topics_to_subscribe, discord_tx).await});
    mqtt_bot.run(mqtt_tx).await;
    info!("Discord bot has stopped.");
}
