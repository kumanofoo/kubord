use log::{info, warn};

use serenity::builder::{
    CreateEmbed,
    CreateCommand,
    CreateCommandOption,
    CreateInteractionResponse,
    CreateInteractionResponseMessage,
    CreateInteractionResponseFollowup,
};
use serenity::model::application::{
    CommandInteraction,
    CommandOptionType,
};
use serenity::client::Context;
use crate::book::BookDbJson;

const DESCRIPTION: &str = "Librarian command";

pub fn description() -> String {
    DESCRIPTION.to_string()
}

pub fn register(slash_command_name: &str) -> CreateCommand {
    CreateCommand::new(slash_command_name)
        .description(DESCRIPTION)
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "title", "Title or ISBN")
                .required(true)
        )
}

pub async fn first_response(ctx: &Context, command: &CommandInteraction, mesg: &str) {
    // First response
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(format!("Searching 『{}』 ...", mesg))
    );

    if let Err(why) = command.create_response(&ctx.http, response).await {
        warn!("Failed to send the first resonse: {:?}", why);
    }      
}

pub async fn handle_command(ctx: &Context, command: &CommandInteraction) -> Option<String> {
    let param = command
        .data
        .options
        .get(0)
        .and_then(|option| option.value.as_str());

    match param {
        Some(k) => {
            first_response(ctx, command, k).await;
            Some(k.to_string())
        },
        None => {
            first_response(ctx, command, "no title!").await;
            None
        },
    }
}

pub fn handle_response(payload: &str) -> Option<CreateInteractionResponseFollowup> {
    let bookdb= match serde_json::from_str::<BookDbJson>(payload) {
        Ok(bookdb) => bookdb,
        Err(why) => {
            warn!("unknown payload: {:?}", why);
            return None;
        }
    };
    let mut embeds = Vec::new();
    for book in bookdb.books {
        let mut embed = CreateEmbed::new();
        info!("{}:{}", book.book_title, book.isbn);
        embed = embed.title(book.book_title).color(0x00FF00);  // Green
        embed = embed.url(format!("https://ndlsearch.ndl.go.jp/search?cs=bib&display=panel&from=0&size=20&f-ht=ndl&f-ht=library&f-isbn={}&f-doc_style=paper", book.isbn));
        for (library_name, details) in book.libraries {
            info!("  library_name: {}", library_name);
            info!("  details: {:?}", details);
            if details.status == "Error" {
                embed = embed.field(library_name, "Error", true);
                continue;
            }
            let locations = match details.libkey {
                Some(loc) => {
                    if loc.is_empty() {
                        embed = embed.field(library_name, "蔵書なし", true);
                        continue;
                    }
                    loc
                },
                None => {
                    embed = embed.field(library_name, "🚫", true);
                    warn!("No libkey");
                    continue;
                }
            };
            for (loc, stat) in locations {
                let value = match &details.reserveurl {
                    Some(url) => format!("[{}]({})", stat, url),
                    None => stat,
                };
                embed = embed.field(format!("{}({})", library_name, loc), value, true);
            }
        }
        embeds.push(embed);
        // A message is limited to 10 embeds.
        if embeds.len() == 9 {
            let embed = CreateEmbed::new()
                .title("Sorry.")
                .description("There isn't enough space to display all the books.")
                .color(0xFFFF00);  // Yellow
            embeds.push(embed);
            break;
        }
    }
    if embeds.len() == 0 {
        let embed = CreateEmbed::new()
            .title(format!("『{}』", bookdb.query))
            .description("No books was found.")
            .color(0x888888);
            embeds.push(embed);
    }
    let followup = CreateInteractionResponseFollowup::new()
        .content(format!("『{}』", bookdb.query))
        .add_embeds(embeds);
    Some(followup)
}
