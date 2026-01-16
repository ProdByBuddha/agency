//! Vocal Cords (Messaging Bridge with bidirectional Ears)
//! 
//! Provides the agency with a voice AND ears on external platforms.
//! Enables proactive notifications and remote command execution.

use teloxide::prelude::*;
use matrix_sdk::{Client as MatrixClient, ruma::{OwnedUserId, OwnedRoomId, events::room::message::{RoomMessageEventContent, MessageType, SyncRoomMessageEvent}}};
use tracing::{info, warn};
use anyhow::Result;
use tokio::sync::OnceCell;
use std::sync::Arc;
use crate::orchestrator::queue::TaskQueue;
use serde_json::json;

pub struct VocalCords {
    tg_bot: Option<Bot>,
    tg_chat_id: Option<ChatId>,
    matrix_client: OnceCell<MatrixClient>,
    matrix_room_id: Option<String>,
}

impl VocalCords {
    /// Initialize the bridge using environment variables
    pub fn new() -> Self {
        // Telegram Config
        let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
        let tg_chat_id_str = std::env::var("TELEGRAM_CHAT_ID").ok();
        
        let tg_bot = tg_token.map(Bot::new);
        let tg_chat_id = tg_chat_id_str.and_then(|id| id.parse::<i64>().ok()).map(ChatId);

        // Matrix Config (Lazy Init)
        let matrix_room_id = std::env::var("MATRIX_ROOM_ID").ok();

        if tg_bot.is_some() && tg_chat_id.is_some() {
            info!("🔊 Vocal Cords: Telegram enabled.");
        }
        
        if matrix_room_id.is_some() {
            info!("🔊 Vocal Cords: Matrix configured (lazy init).");
        }

        Self { 
            tg_bot, 
            tg_chat_id, 
            matrix_client: OnceCell::new(),
            matrix_room_id 
        }
    }

    async fn get_matrix_client(&self) -> Option<&MatrixClient> {
        let homeserver = std::env::var("MATRIX_HOMESERVER").ok()?;
        let user_id_str = std::env::var("MATRIX_USER_ID").ok()?;
        let password = std::env::var("MATRIX_PASSWORD").ok()?;

        self.matrix_client.get_or_try_init(|| async {
            info!("🌐 Initializing Matrix client...");
            let user = <OwnedUserId>::try_from(user_id_str.as_str())
                .map_err(|e| anyhow::anyhow!("Invalid Matrix User ID: {}", e))?;
            
            let client = MatrixClient::builder()
                .homeserver_url(homeserver)
                .build()
                .await?;
            
            client.matrix_auth().login_username(user, &password).send().await?;
            info!("✅ Matrix login successful.");
            Ok::<_, anyhow::Error>(client)
        }).await.ok()
    }

    /// Start listening for messages on all active channels
    pub async fn start_listening(&self, queue: Arc<dyn TaskQueue>) {
        info!("👂 Vocal Cords: Opening ears...");

        // 1. Listen to Telegram
        if let (Some(bot), Some(allowed_chat_id)) = (self.tg_bot.clone(), self.tg_chat_id) {
            let q = queue.clone();
            tokio::spawn(async move {
                let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message, q: Arc<dyn TaskQueue>| async move {
                    if msg.chat.id == allowed_chat_id {
                        if let Some(text) = msg.text() {
                            info!("📥 Received Telegram command: {}", text);
                            let _ = q.enqueue("autonomous_goal", json!(text)).await;
                            let _ = bot.send_message(msg.chat.id, "✅ Command enqueued to Agency.").await;
                        }
                    }
                    respond(())
                });

                Dispatcher::builder(bot, handler)
                    .dependencies(dptree::deps![q])
                    .enable_ctrlc_handler()
                    .build()
                    .dispatch()
                    .await;
            });
        }

        // 2. Listen to Matrix
        if let Some(room_id_str) = self.matrix_room_id.clone() {
            if let Some(client) = self.get_matrix_client().await {
                let client_clone = client.clone();
                let q = queue.clone();
                tokio::spawn(async move {
                    client_clone.add_event_handler(move |ev: SyncRoomMessageEvent, client: MatrixClient| {
                        let q = q.clone();
                        let room_id_str = room_id_str.clone();
                        async move {
                            if let Ok(room_id) = <OwnedRoomId>::try_from(room_id_str.as_str()) {
                                if let Some(room) = client.get_room(&room_id) {
                                    if let Some(original) = ev.as_original() {
                                        if let MessageType::Text(text_content) = &original.content.msgtype {
                                            let text = &text_content.body;
                                            if !text.contains("✅ Command enqueued") {
                                                info!("📥 Received Matrix command: {}", text);
                                                let _ = q.enqueue("autonomous_goal", json!(text)).await;
                                                // Send confirmation
                                                let content = RoomMessageEventContent::text_plain("✅ Command enqueued to Agency.");
                                                let _ = room.send(content).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    });
                    
                    let _ = client_clone.sync(matrix_sdk::config::SyncSettings::default()).await;
                });
            }
        }
    }

    /// Send a proactive message to all active channels
    pub async fn say(&self, message: &str) -> Result<()> {
        // 1. Send to Telegram
        if let (Some(bot), Some(chat_id)) = (&self.tg_bot, self.tg_chat_id) {
            info!("📣 Sending Telegram notification...");
            if let Err(e) = bot.send_message(chat_id, message).await {
                warn!("Telegram notification failed: {}", e);
            }
        }

        // 2. Send to Matrix
        if let Some(room_id_str) = &self.matrix_room_id {
            if let Some(client) = self.get_matrix_client().await {
                info!("📣 Sending Matrix notification...");
                if let Ok(room_id) = <OwnedRoomId>::try_from(room_id_str.as_str()) {
                    if let Some(room) = client.get_room(&room_id) {
                        let content = RoomMessageEventContent::text_plain(message);
                        if let Err(e) = room.send(content).await {
                            warn!("Matrix notification failed: {}", e);
                        }
                    } else {
                        warn!("Matrix: Room {} not found.", room_id_str);
                    }
                } else {
                    warn!("Matrix: Invalid Room ID format: {}", room_id_str);
                }
            }
        }

        Ok(())
    }

    /// Whether any vocal channel is active
    pub fn is_active(&self) -> bool {
        let tg_active = self.tg_bot.is_some() && self.tg_chat_id.is_some();
        let matrix_active = self.matrix_room_id.is_some();
        tg_active || matrix_active
    }
}
