mod notify_controller;
mod users_rep;

use async_mutex::Mutex;
use chrono::{FixedOffset, Local, TimeZone, Timelike};
use notify_controller::{StartEnum, HOUR_FROM, HOUR_TO};
use std::{sync::Arc, time::Duration};
use tokio::{spawn, time::sleep};

use teloxide::{filter_command, prelude::*, utils::command::BotCommands};

use crate::{notify_controller::NotifyController, users_rep::UsersRep};

static ERROR_MSG: &str = "Something go wrong 😫";

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

#[tokio::main]
async fn main() {
    dotenv::from_filename(".env").unwrap();
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
        .branch(commands_handler)
        .branch(dptree::endpoint(handle_message));

    let rep_mutex = Arc::new(Mutex::new(UsersRep::open_or_create("users.db").unwrap()));
    let notify_controller_mutex = Arc::new(Mutex::new(NotifyController::new(bot.clone())));

    {
        let mut notify_controller = notify_controller_mutex.try_lock().unwrap();
        rep_mutex
            .try_lock()
            .unwrap()
            .get_all()
            .iter()
            .for_each(|user_id| {
                notify_controller.start(user_id);
            })
    }

    Dispatcher::builder(bot, messages_handler)
        .enable_ctrlc_handler()
        .dependencies(dptree::deps![rep_mutex, notify_controller_mutex])
        .build()
        .dispatch()
        .await;
}

async fn handle_start_command(
    bot: Bot,
    msg: Message,
    users_rep_mutex: Arc<Mutex<UsersRep>>,
    notify_controller_mutex: Arc<Mutex<NotifyController>>,
) -> ResponseResult<()> {
    let mut rep = users_rep_mutex.lock().await;
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

    match notify_controller.start(&msg.chat.id) {
        StartEnum::Added => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "Notifications sending started!\n\
                    Current timezone: {}\n\
                    Notifications will be sent from {}:00 to {}:00 \
                    every hour untill the \"/done\" command is sent",
                    rep.get(&msg.chat.id).to_string(),
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
    users_rep_mutex: Arc<Mutex<UsersRep>>,
    notify_controller_mutex: Arc<Mutex<NotifyController>>,
) -> ResponseResult<()> {
    let mut users_rep = users_rep_mutex.lock().await;
    match users_rep.rem(&msg.chat.id) {
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
    users_rep_mutex: Arc<Mutex<UsersRep>>,
    notify_controller_mutex: Arc<Mutex<NotifyController>>,
) -> ResponseResult<()> {
    let mut notify_controller = notify_controller_mutex.lock().await;
    match notify_controller.stop(&msg.chat.id) {
        true => {
            spawn(wake_up_tommorow(
                msg.chat.id.clone(),
                5 * 3600,
                Arc::clone(&users_rep_mutex),
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
    users_rep_mutex: Arc<Mutex<UsersRep>>,
    notify_controller_mutex: Arc<Mutex<NotifyController>>,
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

    let rep = users_rep_mutex.lock().await;
    if !rep.exists(&user_id) {
        return;
    }

    let mut controller = notify_controller_mutex.lock().await;
    match controller.start(&user_id) {
        StartEnum::AlreadyExist => {
            log::debug!("Notify task for {} already started", user_id)
        }
        StartEnum::Added => {}
    }
}

async fn handle_change_timezone_command(
    bot: Bot,
    msg: Message,
    users_rep_mutex: Arc<Mutex<UsersRep>>,
) -> ResponseResult<()> {
    let offset = users_rep_mutex.lock().await.get(&msg.chat.id);

    bot.send_message(
        msg.chat.id,
        format!(
            "Current timezone: {}\n\nSend new timezone.\nExamples:\n1. +05:00\n2. -03:00\n3. +03:30",
            offset.to_string()
        ),
    )
    .await?;
    Ok(())
}

async fn handle_message(bot: Bot, msg: Message) -> ResponseResult<()> {
    bot.delete_message(msg.chat.id, msg.id).await?;
    Ok(())
}
