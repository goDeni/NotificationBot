use teloxide::{prelude::*};



#[tokio::main]
async fn main() {
    dotenv::from_filename(".env").unwrap();

    pretty_env_logger::init();
    log::info!("Starting bot...");
    let bot = Bot::from_env();

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        bot.send_dice(msg.chat.id).await?;
        Ok(())
    }).await;
}
