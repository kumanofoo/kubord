use log::{info, warn};
use std::collections::HashMap;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serenity::builder::{
    CreateEmbed,
    CreateCommand,
    CreateCommandOption,
    CreateInteractionResponse,
    CreateInteractionResponseMessage,
    CreateMessage,
};
use serenity::model::timestamp::Timestamp;
use serenity::model::application::{
    CommandInteraction,
    CommandOptionType,
};
use serenity::client::Context;

const DESCRIPTION: &str = "Tracking command";

pub fn description() -> String {
    DESCRIPTION.to_string()
}

pub fn register(slash_command_name: &str) -> CreateCommand {
    println!("slash_command_name: {}", slash_command_name);
    CreateCommand::new(slash_command_name)
        .description(DESCRIPTION)
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", "Add a item to tracking list")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "number", "Tracking Number or URL")
                        .required(true)
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "carrier", "Carrier")
                        .add_string_choice("Japan Post", "jp")
                        .add_string_choice("Yamato", "yamato")
                        .add_string_choice("Sagawa", "sagawa")
                        .required(true)
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "alias", "Nickname of tracking number")
                )
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "delete", "Stop tracking")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "number", "Tracking Number or alias")
                        .required(true)
                )
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "list", "Show tracking numbers")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "number", "Tracking Number or alias")
                )
        )
}

pub async fn first_response(ctx: &Context, command: &CommandInteraction) {
    // First response
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(format!("Ok!"))
    );

    if let Err(why) = command.create_response(&ctx.http, response).await {
        println!("Failed to send the first resonse: {:?}", why);
    }      
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum Carrier {
    Jp,
    Yamato,
    Sagawa,
}

impl Carrier {
    pub fn color(&self) -> u32 {
        match self {
            Carrier::Jp => 0xcc0000,
            Carrier::Yamato => 0xfccf00,
            Carrier::Sagawa => 0x3b499f,
        }
    }
    pub fn logo(&self) -> String {
        match self {
            Carrier::Jp => format!("https://www.japanpost.jp/group/images/index02_img_08.gif"),
            Carrier::Yamato => format!("https://www.kuronekoyamato.co.jp/banner/banner1.gif"),
            Carrier::Sagawa => format!("https://www.sagawa-exp.co.jp/assets/img/help/bnr_transport.gif"),
        }
    }
    pub fn query_url(&self, number: &str) -> String {
        match self {
            Carrier::Jp => format!("https://trackings.post.japanpost.jp/services/srv/search/direct?searchKind=S002&locale=ja&reqCodeNo1={}", number),
            Carrier::Yamato => format!("https://member.kms.kuronekoyamato.co.jp/parcel/detail?pno={}", number),
            Carrier::Sagawa => format!("https://k2k.sagawa-exp.co.jp/p/web/okurijosearch.do?okurijoNo={}", number),
        }
    }
}

impl std::fmt::Display for Carrier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Carrier::Jp => write!(f, "Japan Post"),
            Carrier::Yamato => write!(f, "Yamato"),
            Carrier::Sagawa => write!(f, "Sagawa"),
        }
    }
}

impl Carrier {
    fn from_str(carrier: &str) -> Option<Self> {
        match carrier {
            "Japan Post" | "jp" | "JP" => Some(Carrier::Jp),
            "Yamato" | "yamato" | "YAMATO" => Some(Carrier::Yamato),
            "Sagawa" | "sagawa" | "SAGAWA" => Some(Carrier::Sagawa),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum SubCommand {
    Add { number: String, carrier: Carrier, alias: Option<String> },
    Delete { number: String },
    List { number: Option<String> },
}

impl SubCommand {
    fn from_command_interaction(command: &CommandInteraction) -> Option<Self> {
        let json = serde_json::json!(command.data.options).to_string();
        println!("json: {:?}", json);
        let subcommands_json: Vec<SubCommandJson> = match serde_json::from_str(&json) {
            Ok(subcommand) => subcommand,
            Err(why) => {
                warn!("Unknown tracking subcommand: {}", why);
                return None;
            }
        };

        let subcommand_json = match subcommands_json.get(0) {
            Some(sub) => sub,
            None => {
                warn!("No Subcommands");
                return None;
            }
        };
        println!("subcommand_json: {:?}", subcommand_json);
        match subcommand_json.name.as_str() {
            "add" => {
                let number = match subcommand_json.options
                    .iter().find(|o| o.name == "number").map(|o| o.value.as_str()) {
                        Some(n) => n.to_string(),
                        None => return None,
                    };
                let carrier = match subcommand_json.options
                    .iter().find(|o| o.name == "carrier").map(|o| o.value.as_str()) {
                        Some(n) => match Carrier::from_str(n) {
                            Some(c) => c,
                            None => return None,
                        },
                        None => return None,
                    };
                let alias = subcommand_json.options
                    .iter().find(|o| o.name == "alias").map(|o| o.value.to_string());
                Some(SubCommand::Add {number, carrier, alias})
            },
            "delete" => {
                let number = match subcommand_json.options
                    .iter().find(|o| o.name == "number").map(|o| o.value.as_str()) {
                        Some(n) => n.to_string(),
                        None => return None,
                    };
                Some(SubCommand::Delete {number})
            },
            "list" => {
                let number = subcommand_json.options
                    .iter().find(|o| o.name == "number").map(|o| o.value.to_string());
                Some(SubCommand::List {number})
            },
            _ => {
                warn!("Unknown tracking subcommand: {}", subcommand_json.name);
                return None;
            }
        }
    }

    pub fn from_payload(payload: &str) -> Option<Self> {
        match serde_json::from_str(payload) {
            Ok(subcommand) => Some(subcommand),
            Err(why) => {
                warn!("Failed to parse payload: {}: {}", why, payload);
                return None;
            },
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum SubCommandResponse {
    Add(Result<TrackingItem, String>),
    Delete(Result<String, String>),
    List(Result<HashMap<String, Option<String>>, String>),
}

#[derive(Debug, Deserialize)]
pub struct Options {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
pub struct SubCommandJson {
    name: String,
    options: Vec<Options>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TrackingRecord {
    pub timestamp: i64,
    pub activity: String,
    pub office: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TrackingItem {
    pub item_number: String,
    pub carrier: Carrier,
    pub timestamp: i64,
    pub log: Vec<TrackingRecord>,
    pub alias: Option<String>,
}

pub async fn handle_command(ctx: &Context, command: &CommandInteraction) -> Option<(String, Option<u64>)> {
    first_response(ctx, command).await;
    let subcommand: SubCommand = match SubCommand::from_command_interaction(command) {
        Some(subcommand) => subcommand,
        None => {
            warn!("Unknown tracking subcommand");
            return None;
        },
    };
    let session_timer_duration = match &subcommand {
        SubCommand::Add { .. } => None,
        SubCommand::Delete { .. } => Some(2*60), // 2 minutes
        SubCommand::List { .. } => Some(2*60), // 2 minutes
    };
    let subcommand_str = match serde_json::to_string(&subcommand) {
        Ok(sub) => sub,
        Err(why) => {
            warn!("Failed to create JSON: {}", why);
            return None;
        }
    };
    return Some((subcommand_str, session_timer_duration));
}

pub fn handle_response(payload: &str) -> Option<CreateMessage> {
    let response = match serde_json::from_str::<SubCommandResponse>(payload) {
        Ok(response) => response,
        Err(why) => {
            warn!("unknown payload: {:?}", why);
            return None;
        }
    };
    match response {
        SubCommandResponse::Add(res) => {
            info!("Add response: {:?}", res);
            let tracking_item = match res {
                Ok(t) => t,
                Err(why) => {
                    warn!("Invalid response: {}", why);
                    return None;
                },
            };
            let timestamp = match Timestamp::from_unix_timestamp(tracking_item.timestamp) {
                Ok(ts) => ts,
                Err(why) => {
                    warn!("Invalid timestamp: {}", why);
                    Timestamp::now()
                },
            };
            let mut embed = CreateEmbed::new();
            embed = embed
                .title(format!("{}", tracking_item.carrier))
                .thumbnail(tracking_item.carrier.logo())
                .url(tracking_item.carrier.query_url(&tracking_item.item_number))
                .timestamp(timestamp)
                .color(tracking_item.carrier.color());
            embed = match tracking_item.alias {
                Some(a) => embed.description(format!("Number: {}\nAlias: {}", tracking_item.item_number, a)),
                None => embed.description(format!("Number: {}", tracking_item.item_number)),
            };

            for record in tracking_item.log {
                let datetime_str = match DateTime::from_timestamp(record.timestamp, 0) {
                    Some(dt) => {
                        let jst = FixedOffset::east_opt(9*60*60).unwrap();
                        dt.with_timezone(&jst).format("%Y-%m-%d %H:%M").to_string()
                    },
                    None => {
                        warn!("Invalid timestamp: {}", record.timestamp);
                        "?".to_string()
                    },
                };
                let status = match &record.office {
                    Some(office) => format!("{}({})\n", record.activity, office),
                    None => format!("{}\n", record.activity),
                };
                embed = embed.field(datetime_str, status, false);
            }
            let followup = CreateMessage::new()
                .content(format!("Tracking Status :package:"))
                .add_embed(embed);
            return Some(followup)
        },
        SubCommandResponse::Delete(res) => {
            info!("Delete response: {:?}", res);
            match res {
                Ok(response) => {
                    return Some(CreateMessage::new().content(response));
                },
                Err(why) => {
                    return Some(CreateMessage::new().content(why));
                },
            }
        },
        SubCommandResponse::List(res) => {
            info!("List response: {:?}", res);
            match res {
                Ok(list) => {
                    let numbers = list.iter().map(|(k, v)| {
                        match v {
                            Some(alias) => format!("{} - {}\n", k, alias),
                            None => format!("{}\n", k),
                        }
                    }).collect::<Vec<String>>().concat();
                    if numbers == "" {
                        return Some(CreateMessage::new().content(format!("No Items :pear:")));
                    } else {
                        let embed = CreateEmbed::new()
                            .title(format!("Tracking Items"))
                            .description(numbers)
                            .color(0x00ff00);
                        return Some(CreateMessage::new().add_embed(embed));
                    }
                },
                Err(why) => {
                    return Some(CreateMessage::new().content(format!("{}", why)));
                },
            };
        },
    }
}
