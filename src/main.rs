use std::{
    collections::{HashMap, HashSet},
    env,
    sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex},
};

use lazy_static::lazy_static;
use chrono::{Datelike, Local, TimeZone, Timelike};
use dotenv::dotenv;
use rand::seq::SliceRandom;
use rand::thread_rng;
use teloxide::{
    dispatching::UpdateFilterExt,
    prelude::*,
    utils::command::BotCommands,
};
use rand::prelude::*;

use tokio::time::{sleep, interval_at, Duration, Instant};

/// Our bot commands.
#[derive(teloxide::macros::BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Kumande:")]
enum Command {
    #[command(description = "Štartej")]
    Start,
    #[command(description = "Duloč kdo ga ma")]
    Duloc,
    #[command(description = "Pukaž sttistiko d vidm...")]
    Pukaz,
    #[command(description = "Pošalji")]
    Fora,
    #[command(description = "Pumagej")]
    Help,
}

/// Shared application state.
struct AppState {
    /// Set of chat IDs to send the hourly messages (and jokes) to.
    chats: Mutex<HashSet<ChatId>>,
    /// Counter for hourly messages sent.
    hourly_count: AtomicUsize,
    /// Counter for /duloc commands executed.
    roll_count: AtomicUsize,
}

/// Global counts for names (using lazy_static).
lazy_static! {
    static ref counts: Arc<Mutex<HashMap<String, i32>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init();
    log::info!("Starting bot...");

    // Read the token from the .env file.
    let token = env::var("TELOXIDE_TOKEN")
        .expect("TELOXIDE_TOKEN not set in .env file");
    let bot = Bot::new(token);

    // Initialize the shared state.
    let state = Arc::new(AppState {
        chats: Mutex::new(HashSet::new()),
        hourly_count: AtomicUsize::new(0),
        roll_count: AtomicUsize::new(0),
    });

    // The list of names.
    let names = vec!["Domen", "Tim", "Peter", "Urban", "Matej", "Igor"];

    {
        let mut counts_ref = counts.lock().unwrap();
        for name in names.iter() {
            counts_ref.insert(name.to_string(), 0);
        }
    }

    // Background task: hourly random name message (only during allowed window).
    {
        let bot = bot.clone();
        let state = Arc::clone(&state);
        let names = names.clone();
        tokio::spawn(async move {
            // Wait until we are within the allowed window.
            loop {
                let now = Local::now();
                let current_year = now.year();
                let feb15 = Local.ymd(current_year, 2, 15)
                    .and_hms(0, 0, 0);
                let feb16 = Local.ymd(current_year, 2, 16)
                    .and_hms(0, 0, 0);
                if now < feb15 {
                    let delay = (feb15 - now).to_std().unwrap();
                    log::info!("Waiting {:?} until Feb 15 00:00", delay);
                    sleep(delay).await;
                } else if now >= feb15 && now < feb16 {
                    break; // We're inside the allowed window.
                } else {
                    let next_feb15 = Local.ymd(current_year + 1, 2, 15).and_hms(0, 0, 0);
                    let delay = (next_feb15 - now).to_std().unwrap();
                    log::info!("Past Feb 16; waiting {:?} until next Feb 15", delay);
                    sleep(delay).await;
                }
            }

            // Align to the next full hour.
            let now = Local::now();
            let next_full_hour = now
                .with_minute(0).unwrap()
                .with_second(0).unwrap()
                .with_nanosecond(0).unwrap() + chrono::Duration::hours(1);
            let delay = (next_full_hour - now).to_std().unwrap();
            log::info!("Waiting {:?} until the next full hour", delay);
            sleep(delay).await;

            // Start an hourly ticker.
            let start_instant = Instant::now();
            let mut ticker = interval_at(start_instant, Duration::from_secs(3600));
            loop {
                // Check if still within allowed window.
                let now = Local::now();
                let current_year = now.year();
                let feb16 = Local.ymd(current_year, 2, 16).and_hms(0, 0, 0);
                if now >= feb16 {
                    log::info!("Allowed window ended (Feb 16 00:00), stopping ticker.");
                    break;
                }
                ticker.tick().await;
                let chosen_name = names
                    .choose(&mut thread_rng())
                    .expect("Names list is empty")
                    .to_string();
                state.hourly_count.fetch_add(1, Ordering::Relaxed);
                let chats = {
                    let chats_lock = state.chats.lock().unwrap();
                    chats_lock.clone()
                };
                for chat_id in chats {
                    if let Err(err) = bot.send_message(chat_id, chosen_name.clone()).await {
                        log::error!("Error sending hourly message to chat {}: {:?}", chat_id, err);
                    }
                }
            }
        });
    }

    // NEW: Background task: every minute, fetch a Dad Joke and send it in two parts.
    {
        let bot = bot.clone();
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            // Align to the start of the next minute.
            let now = Local::now();
            let next_minute = now.with_second(0).unwrap().with_nanosecond(0).unwrap() + chrono::Duration::minutes(1);
            let delay = (next_minute - now).to_std().unwrap();
            sleep(delay).await;

            use rand::Rng;

            // Define your minimum and maximum interval (in seconds)
            let min_interval = 30;
            let max_interval = 90;

            loop {
                // Choose a random delay between min_interval and max_interval (inclusive)
                let random_secs = rand::thread_rng().gen_range(min_interval..=max_interval);
                sleep(Duration::from_secs(random_secs)).await;

                // Now perform the API request and process the joke:
                let client = reqwest::Client::new();
                let resp = client
                    .get("https://icanhazdadjoke.com/")
                    .header("Accept", "application/json")
                    .send()
                    .await;
                if let Ok(resp) = resp {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(joke) = json.get("joke").and_then(|v| v.as_str()) {
                            let chats = {
                                let chats_lock = state.chats.lock().unwrap();
                                chats_lock.clone()
                            };
                            // Try to split the joke by the first '?'.
                            let parts: Vec<&str> = joke.splitn(2, '?').collect();
                            if parts.len() == 2 {
                                let setup = parts[0].trim().to_string() + "?";
                                let punchline = parts[1].trim().to_string();
                                // Send the setup to all subscribed chats.
                                for chat_id in chats.iter() {
                                    if let Err(err) = bot.send_message(*chat_id, &setup).await {
                                        log::error!("Error sending joke setup to chat {}: {:?}", chat_id, err);
                                    }
                                }
                                sleep(Duration::from_secs(5)).await;
                                // Send the punchline.
                                for chat_id in chats.iter() {
                                    if let Err(err) = bot.send_message(*chat_id, &punchline).await {
                                        log::error!("Error sending joke punchline to chat {}: {:?}", chat_id, err);
                                    }
                                }
                            } else {
                                // If the joke doesn't contain a '?', send it as one message.
                                for chat_id in chats.iter() {
                                    if let Err(err) = bot.send_message(*chat_id, joke).await {
                                        log::error!("Error sending joke to chat {}: {:?}", chat_id, err);
                                    }
                                }
                            }
                        }
                    }
                }
            }

        });
    }

    // Command handler.
    let handler = Update::filter_message().branch(
        dptree::entry()
            .filter_command::<Command>()
            .endpoint({
                let names = names.clone();
                move |bot: Bot, state: Arc<AppState>, msg: Message, cmd: Command| {
                    let state = Arc::clone(&state);
                    let names = names.clone();
                    async move {
                        match cmd {
                            Command::Help => {
                                bot.send_message(msg.chat.id, Command::descriptions().to_string())
                                    .await?;
                            }
                            Command::Start => {
                                let chat_id = msg.chat.id;
                                {
                                    let mut chats = state.chats.lock().unwrap();
                                    chats.insert(chat_id);
                                }
                                bot.send_message(chat_id, "Tm js puvedou kdaj štart...")
                                    .await?;
                            }
                            Command::Duloc => {
                                let chosen_name = names
                                    .choose(&mut thread_rng())
                                    .expect("Names list is empty")
                                    .to_string();
                                state.roll_count.fetch_add(1, Ordering::Relaxed);
                                {
                                    let mut counts_ref = counts.lock().unwrap();
                                    let count = counts_ref.get_mut(&chosen_name).map_or(&0, |v| v);
                                    let new_count = *count + 1;
                                    counts_ref.insert(chosen_name.clone(), new_count);
                                }
                                bot.send_message(msg.chat.id, chosen_name)
                                    .await?;
                            }
                            Command::Pukaz => {
                                let hourly = state.hourly_count.load(Ordering::Relaxed);
                                let roll = state.roll_count.load(Ordering::Relaxed);
                                let name_stats = {
                                    let counts_ref = counts.lock().unwrap();
                                    let mut s = String::new();
                                    for (name, count) in counts_ref.iter() {
                                        s.push_str(&format!("\t{}: {}x\n", name, count));
                                    }
                                    s
                                };
                                let text = format!(
                                    "Js sm reku {}x\nExtra ste hotl {}x\n\nDoločen:\n{}",
                                    hourly, roll, name_stats
                                );
                                bot.send_message(msg.chat.id, text).await?;
                            }
                            Command::Fora => {
                                // New command: fetch and send a Dad Joke.
                                let client = reqwest::Client::new();
                                let resp = client
                                    .get("https://icanhazdadjoke.com/")
                                    .header("Accept", "application/json")
                                    .send()
                                    .await;
                                if let Ok(resp) = resp {
                                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                                        if let Some(joke) = json.get("joke").and_then(|v| v.as_str()) {
                                            let parts: Vec<&str> = joke.splitn(2, '?').collect();
                                            if parts.len() == 2 {
                                                let setup = format!("{}?", parts[0].trim());
                                                let punchline = parts[1].trim().to_string();
                                                bot.send_message(msg.chat.id, setup).await?;
                                                sleep(Duration::from_secs(5)).await;
                                                bot.send_message(msg.chat.id, punchline).await?;
                                            } else {
                                                bot.send_message(msg.chat.id, joke).await?;
                                            }
                                        } else {
                                            bot.send_message(msg.chat.id, "Nimam nič zdej...").await?;
                                        }
                                    }
                                } else {
                                    bot.send_message(msg.chat.id, "Nimam nič zdej...").await?;
                                }
                            }
                        }
                        respond(())
                    }
                }
            })
    );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
