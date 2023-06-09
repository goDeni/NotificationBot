mod notify_controller;
mod offsets_rep;

use async_mutex::Mutex;
use chrono::{FixedOffset, Local, TimeZone, Timelike};
use notify_controller::{Notification, StartEnum, HOUR_FROM, HOUR_TO};
use regex::Regex;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{spawn, time::sleep};

use teloxide::{
    dispatching::dialogue::InMemStorage, filter_command, prelude::*, utils::command::BotCommands,
};

use crate::{notify_controller::NotificationSender, offsets_rep::OffsetsRepository};

static ERROR_MSG: &str = "Something go wrong 😫";
static TIMEZONE_RE: &str = r"^([+-])([0-2][0-9]):([0-5][0-9])$";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    #[command(description = "Start hotifications sending")]
    Start,
    #[command(description = "Stop hotifications sending")]
    Stop,
    #[command(description = "Stop notifications until tomorrow")]
    Done,
    #[command(description = "Start time zone change dialog")]
    ChangeTimezone,
}

type MyDialogue = Dialogue<State, InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Default)]
enum State {
    #[default]
    RemoveMessages,
    RecieveNewTimezoneOffset,
}

#[tokio::main]
async fn main() {
    {
        let env_file = Path::new(".env");
        if env_file.exists() {
            dotenv::from_filename(".env").unwrap();
        }
    }

    pretty_env_logger::formatted_timed_builder()
        .parse_filters(&std::env::var(&"RUST_LOG").unwrap_or("DEBUG".to_string()))
        .init();

    log::info!("Starting bot...");
    let bot = Bot::from_env();

    let commands_handler = filter_command::<Command, _>()
        .branch(dptree::case![Command::Start].endpoint(handle_start_command))
        .branch(dptree::case![Command::Stop].endpoint(handle_stop_command))
        .branch(dptree::case![Command::Done].endpoint(handle_done_command))
        .branch(dptree::case![Command::ChangeTimezone].endpoint(handle_change_timezone_command));

    let messages_handler = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .branch(commands_handler)
        .branch(dptree::case![State::RemoveMessages].endpoint(handle_message))
        .branch(dptree::case![State::RecieveNewTimezoneOffset].endpoint(handle_new_timezone));

    let offsets_repository = OffsetsRepository::open_or_create("users.db").unwrap();
    let mut notification_sender = Notification::build({
        if let Ok(value) = std::env::var(&"NOTIFICATION_MESSAGE") {
            value
        } else {
            log::warn!("NOTIFICATION_MESSAGE environment variable not set");
            "Notify!".to_string()
        }
    })
    .sender(bot.clone());

    offsets_repository
        .get_all()
        .iter()
        .for_each(|(user_id, offset)| {
            notification_sender.start(user_id, offset.to_owned());
        });

    Dispatcher::builder(bot, messages_handler)
        .enable_ctrlc_handler()
        .dependencies(dptree::deps![
            Arc::new(Mutex::new(offsets_repository)),
            Arc::new(Mutex::new(notification_sender)),
            InMemStorage::<State>::new()
        ])
        .build()
        .dispatch()
        .await;
}

async fn handle_start_command(
    bot: Bot,
    msg: Message,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    notify_controller_mutex: Arc<Mutex<NotificationSender>>,
    dialogue: MyDialogue,
) -> HandlerResult {
    dialogue.exit().await?;

    let mut rep = offsets_rep_mutex.lock().await;
    let mut notify_controller = notify_controller_mutex.lock().await;

    if !rep.exists(&msg.chat.id) {
        log::debug!("Adding user {}", msg.chat.id);
        match rep.add(&msg.chat.id) {
            Ok(_) => {
                log::info!("Added user in repo: {}", msg.chat.id);
            }
            Err(err) => {
                log::error!("Failed to add {} user {}", err, msg.chat.id);
                bot.send_message(msg.chat.id, ERROR_MSG).await?;
                return Ok(());
            }
        }
    } else {
        log::debug!("User already exist {}", msg.chat.id);
    }

    let offset = rep.get(&msg.chat.id).unwrap();
    match notify_controller.start(&msg.chat.id, offset) {
        StartEnum::Added => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "Notifications sending started!\n\
                    Current timezone: {}\n\
                    Notifications will be sent from {}:00 to {}:00 \
                    every hour untill the \"/done\" command is sent",
                    offset.to_string(),
                    HOUR_FROM,
                    HOUR_TO
                ),
            )
            .await?;
        }
        StartEnum::AlreadyExist => {
            bot.send_message(msg.chat.id, "Already started!").await?;
        }
    };
    Ok(())
}

async fn handle_stop_command(
    bot: Bot,
    msg: Message,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    notify_controller_mutex: Arc<Mutex<NotificationSender>>,
    dialogue: MyDialogue,
) -> HandlerResult {
    dialogue.exit().await?;

    let mut offsets_rep = offsets_rep_mutex.lock().await;
    match offsets_rep.rem(&msg.chat.id) {
        Ok(true) => {
            let mut notify_controller = notify_controller_mutex.lock().await;
            notify_controller.stop(&msg.chat.id);

            bot.send_message(msg.chat.id, "Stoped!").await?;
        }
        Ok(false) => {
            bot.send_message(msg.chat.id, "Nothing to stop").await?;
        }
        Err(err) => {
            log::error!("Unable to remove user {}: {}", msg.chat.id, err);
            bot.send_message(msg.chat.id, ERROR_MSG).await?;
        }
    }

    Ok(())
}

async fn handle_done_command(
    bot: Bot,
    msg: Message,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    notify_controller_mutex: Arc<Mutex<NotificationSender>>,
    dialogue: MyDialogue,
) -> HandlerResult {
    dialogue.exit().await?;

    let mut notify_controller = notify_controller_mutex.lock().await;
    match notify_controller.stop(&msg.chat.id) {
        true => {
            spawn(wake_up_tommorow(
                msg.chat.id.clone(),
                5 * 3600,
                Arc::clone(&offsets_rep_mutex),
                Arc::clone(&notify_controller_mutex),
            ));
            bot.send_message(msg.chat.id, "Notifications delayed until tomorrow")
                .await?;
        }
        false => {
            bot.send_message(msg.chat.id, "Nothing to delay").await?;
        }
    }

    Ok(())
}

async fn wake_up_tommorow(
    user_id: ChatId,
    offset: i32,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    notify_controller_mutex: Arc<Mutex<NotificationSender>>,
) {
    let sleep_time = {
        let date = FixedOffset::east_opt(offset)
            .expect(&format!("Invalid user {} offset {}", user_id, offset))
            .from_utc_datetime(&Local::now().naive_utc());

        u64::from((((24 - date.hour()) * 60) - date.minute()) * 60)
    };

    log::info!(
        "Started \"wake up tommorow\" task for {}, we will sleep {} seconds",
        user_id,
        sleep_time
    );
    sleep(Duration::from_secs(sleep_time)).await;

    let rep = offsets_rep_mutex.lock().await;
    match rep.get(&user_id) {
        Some(offset) => {
            let mut controller = notify_controller_mutex.lock().await;
            match controller.start(&user_id, offset) {
                StartEnum::AlreadyExist => {
                    log::debug!("Notify task for {} already started", user_id)
                }
                StartEnum::Added => {}
            }
        }
        None => {
            log::info!(
                "Unable to wake up because user {} offset doesn't exist",
                user_id
            )
        }
    }
}

async fn handle_change_timezone_command(
    bot: Bot,
    msg: Message,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    dialogue: MyDialogue,
) -> HandlerResult {
    match offsets_rep_mutex.lock().await.get(&msg.chat.id) {
        Some(offset) => {
            dialogue.update(State::RecieveNewTimezoneOffset).await?;
            bot.send_message(
                msg.chat.id,
                format!(
                    "Current timezone: {}\n\nSend new timezone.\nExamples:\n1. +05:00\n2. -03:00\n3. +03:30",
                    offset.to_string()
                ),
            )
            .await?;
        }
        None => {
            bot.send_message(
                msg.chat.id,
                "Timezone cannot be changed while notifications are disabled",
            )
            .await?;
        }
    }
    Ok(())
}

async fn handle_message(bot: Bot, msg: Message) -> HandlerResult {
    bot.delete_message(msg.chat.id, msg.id).await?;
    Ok(())
}

async fn handle_new_timezone(
    bot: Bot,
    msg: Message,
    dialogue: MyDialogue,
    offsets_rep_mutex: Arc<Mutex<OffsetsRepository>>,
    notify_controller_mutex: Arc<Mutex<NotificationSender>>,
) -> HandlerResult {
    let message_text = msg
        .text()
        .expect("Unable to get text in message handler")
        .trim();
    let timezone_regex = Regex::new(TIMEZONE_RE).unwrap();

    if !timezone_regex.is_match(message_text) {
        bot.send_message(msg.chat.id, "Invalid timezone").await?;
        return Ok(());
    }
    let captures = timezone_regex.captures(message_text).unwrap();

    let secs = {
        let hours = captures[2].parse::<i32>().unwrap();
        let minutes = captures[3].parse::<i32>().unwrap();

        hours * 3600 + minutes * 60
    };

    let fixed_offset = match &captures[1] {
        "+" => FixedOffset::east_opt(secs).unwrap(),
        "-" => FixedOffset::west_opt(secs).unwrap(),
        // tests must cover that
        _ => {
            unreachable!()
        }
    };

    let mut offsets_rep = offsets_rep_mutex.lock().await;
    let mut controller = notify_controller_mutex.lock().await;

    match offsets_rep.set(&msg.chat.id, &fixed_offset) {
        Ok(_) => {
            controller.stop(&msg.chat.id);
            controller.start(&msg.chat.id, fixed_offset);

            bot.send_message(
                msg.chat.id,
                format!("Timezone is changed: {}", fixed_offset.to_string()),
            )
            .await?;
            dialogue.exit().await?;
        }
        Err(err) => {
            log::error!(
                "Failed timezone update {}: {}",
                fixed_offset.to_string(),
                err
            );
            bot.send_message(msg.chat.id, ERROR_MSG).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use crate::TIMEZONE_RE;

    #[test]
    fn test_valid_timezone_regex() {
        let regex = Regex::new(TIMEZONE_RE).unwrap();

        // UTC-23:59 to UTC+23:59
        for sign in ["-", "+"] {
            for hour in 0..2 {
                for minute in 0..59 {
                    let str_timezone = format!("{}{:0>2}:{:0>2}", sign, hour, minute);
                    assert!(
                        regex.is_match(&str_timezone),
                        "Unable to match {}",
                        str_timezone
                    );
                }
            }
        }
    }

    #[test]
    fn test_invalid_timezone_regex() {
        let regex = Regex::new(TIMEZONE_RE).unwrap();

        assert!(!regex.is_match("-03:00:00"));
        assert!(!regex.is_match("+03:00:00"));

        assert!(!regex.is_match("-003:00"));
        assert!(!regex.is_match("+003:00"));

        assert!(!regex.is_match("-3:00"));
        assert!(!regex.is_match("+3:00"));

        assert!(!regex.is_match("-03:1"));
        assert!(!regex.is_match("+03:2"));

        assert!(!regex.is_match("-33:00"));
        assert!(!regex.is_match("+33:00"));

        assert!(!regex.is_match("-03:60"));
        assert!(!regex.is_match("+03:60"));

        assert!(!regex.is_match("23:59"));
        assert!(!regex.is_match("plus23:59"));
        assert!(!regex.is_match(" +23:59 "));
    }

    #[test]
    fn test_timezone_regex_groups() {
        let regex = Regex::new(TIMEZONE_RE).unwrap();

        // UTC-23:59 to UTC+23:59
        for sign in ["-", "+"] {
            for hour in 0..2 {
                for minute in 0..59 {
                    let str_timezone = format!("{}{:0>2}:{:0>2}", sign, hour, minute);
                    let captures = regex.captures(&str_timezone);

                    assert!(!captures.is_none(), "Unable to match {}", str_timezone);
                    let captures =
                        captures.expect(&format!("Can't get captures for {}", str_timezone));

                    let matched_sign = captures
                        .get(1)
                        .expect(&format!("Can't get sign for {}", str_timezone))
                        .as_str();
                    assert_eq!(
                        matched_sign, sign,
                        "Got invalid sign {}: {}",
                        str_timezone, matched_sign
                    );

                    let matched_hour = captures
                        .get(2)
                        .expect(&format!("Can't get hours for {}", str_timezone))
                        .as_str();
                    assert_eq!(
                        matched_hour,
                        format!("{:0>2}", hour),
                        "Got invalid hour {}: {}",
                        str_timezone,
                        matched_hour
                    );

                    let matched_minute = captures
                        .get(3)
                        .expect(&format!("Can't get minutes for {}", str_timezone))
                        .as_str();
                    assert_eq!(
                        matched_minute,
                        format!("{:0>2}", minute),
                        "Got invalid minutes {}: {}",
                        str_timezone,
                        matched_minute
                    );
                }
            }
        }
    }
}
