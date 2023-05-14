use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{FixedOffset, Local, TimeZone, Timelike};
use teloxide::{requests::Requester, types::ChatId, Bot};
use tokio::{spawn, task::JoinHandle, time::sleep};

pub struct NotifyController {
    tasks_map: HashMap<ChatId, JoinHandle<()>>,
    bot: Arc<Bot>,
}

pub enum StartEnum {
    Added,
    AlreadyExist,
}

impl NotifyController {
    pub fn new(bot: Bot) -> NotifyController {
        NotifyController {
            tasks_map: HashMap::new(),
            bot: Arc::new(bot),
        }
    }

    pub fn start(&mut self, user_id: &ChatId) -> StartEnum {
        if self.tasks_map.contains_key(user_id) {
            return StartEnum::AlreadyExist;
        }

        let task = spawn(notify_task(user_id.clone(), Arc::clone(&self.bot), 5 * 3600));
        self.tasks_map.insert(user_id.clone(), task);

        log::debug!("Added task {}", user_id);

        StartEnum::Added
    }

    pub fn stop(&mut self, user_id: &ChatId) {
        if !self.tasks_map.contains_key(user_id) {
            return;
        }

        let task = self.tasks_map.remove(user_id).unwrap();
        task.abort();
        log::debug!("Stopped {} task", user_id);
    }
}

async fn notify_task(user_id: ChatId, bot: Arc<Bot>, offset: i32) {
    let fixed_offset = FixedOffset::east_opt(offset).expect(
        &format!("Invalid user {} offset {}", user_id, offset)
    );

    let offset_str = {
        let mut sign = "+";
        if fixed_offset.local_minus_utc().is_negative() {
            sign = "-";
        }

        let offset = fixed_offset.local_minus_utc().abs();
        let hours = offset / 60 / 60;
        let mins = offset / 60 - hours * 60;

        format!("{}\"{} hours {} mins\"", sign, hours, mins)
    };

    let send_message = || async {
        match bot.send_message(user_id, "Notify!").await {
            Ok(_) => {
                return true;
            }
            Err(err) => {
                log::error!("Failed message send {}: {}", user_id, err);
                return false;
            }
        }
    };

    log::debug!("Started notification task for {}!", user_id);
    loop {
        let date = fixed_offset.from_utc_datetime(&Local::now().naive_utc());

        if date.hour() >= 9 && date.hour() < 18 {
            match send_message().await {
                false => {
                    log::error!("Message for {} didn't sent!", user_id);
                    sleep(Duration::from_secs(60)).await;
                    continue;
                }
                true => {
                    log::debug!("Message for {} sent!", user_id);
                }
            }
        } else {
            log::debug!("Non-working hours for user {} with offset {}", user_id, offset_str)
        }

        let sleep_time = u64::from(3600 - (date.minute() * 60 + date.second()));
        log::debug!(
            "Sleep time {} seconds ({} minutes) for user {} offset {}",
            sleep_time,
            sleep_time / 60,
            user_id,
            offset_str,
        );
        sleep(Duration::from_secs(sleep_time)).await;
    }
}