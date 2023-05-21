use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Timelike, Weekday};
use teloxide::{requests::Requester, types::ChatId, Bot};
use tokio::{spawn, task::JoinHandle, time::sleep as async_sleep};

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

    return result.trim().to_string();
}

fn its_working_time(date: DateTime<FixedOffset>) -> bool {
    match (date.weekday(), date.hour()) {
        (Weekday::Sat | Weekday::Sun, _) => false,
        (_, hour) => hour >= HOUR_FROM && hour < HOUR_TO,
    }
}

fn get_sleep_time(date: DateTime<FixedOffset>) -> Duration {
    let days = match date.weekday() {
        Weekday::Sat => 2,
        Weekday::Sun => 1,
        _ => 0,
    };

    let hours: u32;
    if days == 0 {
        if date.hour() < HOUR_FROM {
            hours = HOUR_FROM - date.hour();
        } else if date.hour() >= HOUR_TO {
            hours = 24 - date.hour() + HOUR_FROM;
        } else {
            hours = 1;
        }
    } else if date.hour() < HOUR_FROM {
        hours = 24 * days + HOUR_FROM;
    } else {
        hours = 24 * days - (date.hour() - HOUR_FROM);
    }

    let mut minutes: u32 = hours * 60;
    if date.minute() > 0 {
        minutes -= date.minute();
    }

    let mut seconds = minutes * 60;
    if date.second() > 0 {
        seconds -= date.second();
    }

    Duration::from_secs(u64::from(seconds))
}

async fn notify_task(user_id: ChatId, bot: Arc<Bot>, fixed_offset: FixedOffset, message: String) {
    let get_user_date = || fixed_offset.from_utc_datetime(&Local::now().naive_utc());
    let send_notification = || async {
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
    let sleep = |duration: Duration| {
        log::debug!(
            "Sleep time {}. user_id={}, offset={}",
            format_seconds(duration.as_secs()),
            user_id,
            fixed_offset.to_string(),
        );
        async_sleep(duration)
    };

    log::debug!("Started notification task for {}!", user_id);
    loop {
        {
            let date = get_user_date();
            if !its_working_time(date) {
                sleep(get_sleep_time(date)).await;
            }
        }

        sleep(match send_notification().await {
            true => get_sleep_time(get_user_date()),
            false => Duration::from_secs(60),
        })
        .await;

        if !its_working_time(get_user_date()) {
            log::debug!(
                "Sending today's last message for {} {}",
                user_id,
                fixed_offset.to_string()
            );
            send_notification().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::notify_controller::{
        format_seconds, get_sleep_time, its_working_time, HOUR_FROM, HOUR_TO,
    };
    use chrono::{DateTime, FixedOffset, TimeZone, Utc};

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

    fn get_date(day: u32, hour: u32, min: u32, secs: u32) -> DateTime<FixedOffset> {
        return FixedOffset::east_opt(0).unwrap().from_utc_datetime(
            &Utc.with_ymd_and_hms(2023, 05, day, hour, min, secs)
                .unwrap()
                .naive_utc(),
        );
    }

    #[test]
    fn test_its_working_hours() {
        for minute in 0..=59 {
            for second in 0..=59 {
                for hour in HOUR_FROM..HOUR_TO {
                    assert!(its_working_time(get_date(1, hour, minute, second)));
                }

                for hour in 0..HOUR_FROM {
                    assert!(!its_working_time(get_date(1, hour, minute, second)));
                }

                for hour in HOUR_TO..24 {
                    assert!(!its_working_time(get_date(1, hour, minute, second)));
                }

                for hour in 0..24 {
                    assert!(!its_working_time(get_date(6, hour, minute, second)));
                    assert!(!its_working_time(get_date(7, hour, minute, second)));
                }
            }
        }
    }

    #[test]
    fn test_sleep_time_in_working_hours() {
        for hour in HOUR_FROM..=(HOUR_TO - 1) {
            for minute in 0..=59 {
                for second in 0..=59 {
                    assert_eq!(
                        get_sleep_time(get_date(1, hour, minute, second)).as_secs(),
                        u64::from(3600 - minute * 60 - second),
                        "hour={}, minute={}, second={}",
                        hour,
                        minute,
                        second
                    );
                }
            }
        }
    }

    #[test]
    fn test_sleep_time_before_working_hours() {
        for hour_offset in 1..=HOUR_FROM {
            for minute in 0..=59 {
                for second in 0..=59 {
                    assert_eq!(
                        get_sleep_time(get_date(1, HOUR_FROM - hour_offset, minute, second))
                            .as_secs(),
                        u64::from(3600 * hour_offset - minute * 60 - second),
                    );
                }
            }
        }
    }

    #[test]
    fn test_sleep_time_after_working_hours() {
        for hour in HOUR_TO..=23 {
            for minute in 0..=59 {
                for second in 0..=59 {
                    let sleep_time = get_sleep_time(get_date(1, hour, minute, second)).as_secs();
                    assert_eq!(
                        sleep_time,
                        u64::from((24 - hour + HOUR_FROM) * 3600 - minute * 60 - second),
                        "hour={}, minute={}, second={}, sleep_time=\"{}\"",
                        hour,
                        minute,
                        second,
                        format_seconds(sleep_time)
                    );
                }
            }
        }
    }

    #[test]
    fn test_sleep_time_on_weekends_before_hour_from() {
        for day in 6..=7 {
            for hour in 0..=(HOUR_FROM - 1) {
                for minute in 0..=59 {
                    for second in 0..=59 {
                        let sleep_time =
                            get_sleep_time(get_date(day, hour, minute, second)).as_secs();
                        let expected =
                            u64::from(((24 * (8 - day) + HOUR_FROM) * 60 - minute) * 60 - second);
                        assert_eq!(
                            sleep_time,
                            expected,
                            "day={}, hour={}, minute={}, second={} got=\"{}\", expected=\"{}\"",
                            day,
                            hour,
                            minute,
                            second,
                            format_seconds(sleep_time),
                            format_seconds(expected)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_sleep_time_on_weekends_in_after_hour_from() {
        for day in 6..=7 {
            for hour in HOUR_FROM..=23 {
                for minute in 0..=59 {
                    for second in 0..=59 {
                        let sleep_time =
                            get_sleep_time(get_date(day, hour, minute, second)).as_secs();
                        let expected = u64::from(
                            ((24 * (8 - day) - (hour - HOUR_FROM)) * 60 - minute) * 60 - second,
                        );
                        assert_eq!(
                            sleep_time,
                            expected,
                            "day={}, hour={}, minute={}, second={} got=\"{}\", expected=\"{}\"",
                            day,
                            hour,
                            minute,
                            second,
                            format_seconds(sleep_time),
                            format_seconds(expected)
                        );
                    }
                }
            }
        }
    }
}
