use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{FixedOffset, Local, TimeZone, Timelike};
use teloxide::{requests::Requester, types::ChatId, Bot};
use tokio::{spawn, task::JoinHandle, time::sleep};

pub const HOUR_FROM: u32 = 9;
pub const HOUR_TO: u32 = 18;

pub struct NotificationSender {
    notify_tasks_map: HashMap<ChatId, JoinHandle<()>>,
    bot: Arc<Bot>,
    notification: Notification,
}

pub enum StartEnum {
    Added,
    AlreadyExist,
}

pub struct Notification(String);
impl Notification {
    pub fn build(message: String) -> Notification {
        Notification(message)
    }

    pub fn sender(self, bot: Bot) -> NotificationSender {
        NotificationSender::new(bot, self)
    }

    pub fn message(&self) -> &String {
        return &self.0;
    }
}

impl NotificationSender {
    pub fn new(bot: Bot, notification: Notification) -> NotificationSender {
        NotificationSender {
            notify_tasks_map: HashMap::new(),
            bot: Arc::new(bot),
            notification: notification,
        }
    }

    pub fn start(&mut self, user_id: &ChatId, offset: FixedOffset) -> StartEnum {
        if self.notify_tasks_map.contains_key(user_id) {
            return StartEnum::AlreadyExist;
        }

        let task = spawn(notify_task(
            user_id.clone(),
            Arc::clone(&self.bot),
            offset,
            self.notification.message().to_owned(),
        ));
        self.notify_tasks_map.insert(user_id.clone(), task);

        log::debug!("Added notify task {}", user_id);

        StartEnum::Added
    }

    pub fn stop(&mut self, user_id: &ChatId) -> bool {
        if !self.notify_tasks_map.contains_key(user_id) {
            return false;
        }

        let task = self.notify_tasks_map.remove(user_id).unwrap();
        task.abort();
        log::debug!("Stopped {} notify task", user_id);
        return true;
    }
}

fn format_seconds(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = seconds / 60 - hours * 60;

    let seconds = seconds - minutes * 60 - hours * 3600;

    let mut result = String::new();
    if hours > 0 {
        result += &format!("{} hours ", hours);
    }
    if minutes > 0 {
        result += &format!("{} minutes ", minutes);
    }
    if seconds > 0 || (hours == 0 && minutes == 0) {
        result += &format!("{} seconds ", seconds);
    }

    return result.trim().to_string()
}

async fn notify_task(user_id: ChatId, bot: Arc<Bot>, fixed_offset: FixedOffset, message: String) {
    let get_date = || fixed_offset.from_utc_datetime(&Local::now().naive_utc());
    let send_message = || async {
        match bot
            .send_message(
                user_id,
                format!(
                    "{}\n\n{}",
                    message, "Send the \"/done\" command to turn off notifications until tomorrow"
                ),
            )
            .await
        {
            Ok(_) => {
                log::debug!("Notification message for {} sent!", user_id);
                return true;
            }
            Err(err) => {
                log::error!("Notification message for {} didn't sent: {}", user_id, err);
                return false;
            }
        }
    };

    log::debug!("Started notification task for {}!", user_id);
    loop {
        let date = get_date();
        let sleep_time = {
            if date.hour() < HOUR_FROM {
                u64::from((HOUR_FROM - date.hour()) * 60 - date.minute()) * 60
            } else {
                if date.hour() >= HOUR_TO {
                    u64::from((((24 - date.hour() + HOUR_FROM) * 60) - date.minute()) * 60)
                } else {
                    0
                }
            }
        };
        if sleep_time > 0 {
            log::debug!(
                "Waiting for working hours ({}). user_id={} offset={}",
                format_seconds(sleep_time),
                user_id,
                fixed_offset.to_string(),
            );
            sleep(Duration::from_secs(sleep_time)).await;
        }

        loop {
            let date = get_date();
            if date.hour() >= HOUR_TO {
                break;
            }

            let sleep_time = match send_message().await {
                true => {u64::from(((59 - date.minute()) * 60) + (60 - date.second()))}
                false => {60}
            };

            log::debug!(
                "Sleep time {}. user_id={}", 
                format_seconds(sleep_time),
                user_id,
            );
            sleep(Duration::from_secs(sleep_time)).await;
        }
        send_message().await;
    }
}


#[cfg(test)]
mod tests {
    use crate::notify_controller::format_seconds;

    #[test]
    fn test_format_seconds() {
        assert_eq!(format_seconds(0), "0 seconds");

        assert_eq!(format_seconds(59), "59 seconds");
        assert_eq!(format_seconds(60), "1 minutes");
        assert_eq!(format_seconds(61), "1 minutes 1 seconds");
        assert_eq!(format_seconds(70), "1 minutes 10 seconds");
        assert_eq!(
            format_seconds(3600 * 5 + 3 * 60 + 33), 
            "5 hours 3 minutes 33 seconds"
        );
    }
}