// Subscribe AMeDAS and store the payload into a cache.
// /amedas => return the cache.
// /amedas SCHEDULED_TIME => return the cache at SCHEDULED_TIME.

use log::{info, warn};
use std::collections::HashMap;
use std::sync::{RwLock, LazyLock};
use chrono::{DateTime, Local, Timelike};
use serenity::all::{
    Colour, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage
};
use serenity::builder::CreateCommand;
use serenity::model::application::CommandInteraction;
use serenity::client::Context;
use crate::mqtt::MQTTAmedas;

// ⣀⣤⣶⣿ ⠉⠛⠿⣿
pub const LEVEL_DOT_EMOJI: [char; 5] = ['_', '⣀', '⣤', '⣶', '⣿'];
// ▁▂▃▄▅▆▇█
pub const LEVEL_BOX_EMOJI: [char; 5] = ['⣀', '▂', '▄', '▆', '█'];

const PRECIPITATION_MM_LEVEL_TAB: [f32; 4] = [1.0, 2.0, 10.0, 30.0];

pub fn rain_level_meter(precipitation_mm: f32) -> char {
    for (i, level) in PRECIPITATION_MM_LEVEL_TAB.iter().enumerate() {
        if precipitation_mm < *level {
            return LEVEL_BOX_EMOJI[i];
        }
    }
    return LEVEL_BOX_EMOJI[LEVEL_BOX_EMOJI.len() -1];
}

pub fn snow_level_meter(snow_cm: f32) -> char {
    for (i, level) in PRECIPITATION_MM_LEVEL_TAB.iter().enumerate() {
        if snow_cm/10.0 < *level {
            return LEVEL_DOT_EMOJI[i];
        }
    }
    return LEVEL_BOX_EMOJI[LEVEL_BOX_EMOJI.len() -1];
}

static AMEDAS_STORE: LazyLock<RwLock<HashMap<String, MQTTAmedas>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub static WEATHER_CODE_PNG: LazyLock<RwLock<HashMap<u32, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const DESCRIPTION: &str = "Weather Forecast";

pub fn description() -> String {
    DESCRIPTION.to_string()
}

pub fn register(slash_command_name: &str) -> CreateCommand {
    CreateCommand::new(slash_command_name)
        .description(DESCRIPTION)
}

pub async fn first_response(ctx: &Context, command: &CommandInteraction) {
    // First response
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(format!("Querying AMeDAS ..."))
    );

    if let Err(why) = command.create_response(&ctx.http, response).await {
        warn!("Failed to send the first response: {:?}", why);
    }
}

pub async fn handle_command(ctx: &Context, command: &CommandInteraction) {
    first_response(ctx, command).await;
    
    let mut embeds = Vec::new();
    let store = AMEDAS_STORE.read().unwrap().clone();
    for (_device_id, mqtt_amedas) in store {
        let data = &mqtt_amedas.data;
        let description = format!("{}°Ｃ {}%\n:umbrella: {} {}\n{} {}m/s {}m",
                                  data.temp_c, data.humidity_percent,
                                  rain_level_meter(data.precipitation1h),
                                  data.snow1h.map_or("".to_string(), |v| format!(":snowman2: {}", snow_level_meter(v).to_string())),
                                  data.wind_direction_emoji, data.wind_mps,
                                  data.visibility_m.map_or("-".to_string(), |v| v.to_string()));
        let author = CreateEmbedAuthor::new(format!("📡 {}", mqtt_amedas.station.1));
        let updated = match DateTime::parse_from_rfc3339(&mqtt_amedas.latest_time) {
            Ok(dt) => {
                // "Update 09:27 on Wed 3 Dec 2025"
                dt.format("Update %H:%M on %a %d %b %Y").to_string()
            },
            Err(why) => {
                warn!("Invalid timestamp {}: {}", mqtt_amedas.latest_time, why);
                "?".to_string()
            },
        };
        let footer = CreateEmbedFooter::new(updated);
        let thumbnail = match data.weather {
            Some(w) => {
                let hour = Local::now().hour();
                let mut code = w;
                if  w == 0 && (hour < 5 || hour > 19) {
                    code = 100;
                }
                let code_png = WEATHER_CODE_PNG.read().unwrap();
                match code_png.get(&code) {
                    Some(url) => url.clone(),
                    None => "".to_string(),
                }
            },
            None => "".to_string(),
        };
        let embed = CreateEmbed::new()
            //.title(data.weather_discord_emoji.clone())
            .title(description)
            .author(author)
            .thumbnail(thumbnail)
            //.description(description)
            .color(Colour::from_rgb(244, 213, 42))
            .footer(footer);

        embeds.push(embed);
    }
    let followup = CreateInteractionResponseFollowup::new()
        .content(format!("AMeDAS"))
        .add_embeds(embeds);
    if let Err(why) = command.create_followup(&ctx.http, followup).await {
        warn!("Failed to send followup message: {}", why);
    }
}

pub fn store_amedas(device_id: &str, payload: &str) {
    match serde_json::from_str::<MQTTAmedas>(payload) {
        Ok(amedas) => {
            let mut store = AMEDAS_STORE.write().unwrap();
            store.insert(device_id.to_string(), amedas);
        },
        Err(why) => {
            warn!("unknown payload of {}: {:?}", device_id, why);
            return;
        },
    };
    info!("Store latest AMeDAS data: {}", payload);
}
