//! Vocal Cords (Messaging Bridge)
//! 
//! Provides the agency with a voice on external platforms like Telegram.
//! Enables proactive notifications and mobile interaction.

use teloxide::prelude::*;
use std::sync::Arc;
use tracing::{info, error};
use anyhow::Result;

pub struct VocalCords {
    bot: Option<Bot>,
    chat_id: Option<ChatId>,
}

impl VocalCords {
    /// Initialize the bridge using environment variables
    pub fn new() -> Self {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
        let chat_id_str = std::env::var("TELEGRAM_CHAT_ID").ok();
        
        let bot = token.map(Bot::new);
        let chat_id = chat_id_str.and_then(|id| id.parse::<i64>().ok()).map(ChatId);

        if bot.is_some() && chat_id.is_some() {
            info!("🔊 Vocal Cords initialized via Telegram.");
        } else {
            info!("🔇 Vocal Cords dormant (TELEGRAM_BOT_TOKEN/CHAT_ID not set).");
        }

        Self { bot, chat_id }
    }

    /// Send a proactive message to the user
    pub async fn say(&self, message: &str) -> Result<()> {
        if let (Some(bot), Some(chat_id)) = (&self.bot, self.chat_id) {
            info!("📣 Sending Telegram notification...");
            bot.send_message(chat_id, message).await
                .map_err(|e| anyhow::anyhow!("Telegram error: {}", e))?;
        }
        Ok(())
    }

    /// Whether the vocal cords are currently active
    pub fn is_active(&self) -> bool {
        self.bot.is_some() && self.chat_id.is_some()
    }
}
