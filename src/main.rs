mod config;
mod waifu;
mod hikari;
mod scanner;

use chrono::{Datelike, Utc};
use config::Store;
use serenity::async_trait;
use serenity::model::channel::{Message, ReactionType};
use serenity::model::gateway::Ready;
use serenity::model::guild::Member;
use serenity::model::id::{ChannelId, RoleId, UserId};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::Color;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio::sync::RwLock;

use waifu::{add_waifu_to_embed, get_ssr_color, WaifuTier, PULL_COST};
use hikari::{Hikari, HikariConfig, HikariHandler};
use scanner::{HikariScanner, HikariScannerHandler};

// Bot struct
struct Bot {
    store: Arc<Store>,
    hikari: Arc<RwLock<Hikari>>,
    scanner: Arc<HikariScanner>,
}

// Error handling helper for commands
async fn respond_error(ctx: &Context, channel_id: ChannelId, error_message: &str) {
    if let Err(e) = channel_id.say(&ctx.http, error_message).await {
        println!("Failed to send error message: {:?}", e);
    }
}

// Success response helper
async fn respond_success(ctx: &Context, channel_id: ChannelId, success_message: &str) {
    if let Err(e) = channel_id.say(&ctx.http, success_message).await {
        println!("Failed to send success message: {:?}", e);
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
        println!("Bot ID: {}", ready.user.id);
        println!("Ready to handle DMs and messages!");

        // Initialize the gacha system
        match self.store.init_gacha_system().await {
            Ok(_) => println!("Gacha system initialized successfully"),
            Err(e) => println!("Error initializing gacha system: {:?}", e),
        }

        // Initialize scanner background tasks
        {
            let scanner = self.scanner.clone();
            let ctx_arc = Arc::new(ctx.clone());
            tokio::spawn(async move {
                println!("Starting scanner background tasks");
                scanner.start_background_tasks(ctx_arc).await;
                println!("Scanner background tasks initialized");
            });
        }

        // Set up scheduled tasks
        let store_clone = Arc::clone(&self.store);
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(60 * 60 * 24)); // Once per day
            loop {
                interval.tick().await;

                // Create a daily backup
                if let Err(e) = store_clone.backup_database().await {
                    println!("Error creating daily backup: {:?}", e);
                }

                // Check if it's the first of the month for monthly rotation
                let now = Utc::now();
                if now.day() == 1 {
                    let config = store_clone.get_config().await;
                    if config.monthly_rotation {
                        store_clone.reset_monthly_stats().await;
                    }
                }
            }
        });
    }

    async fn guild_member_addition(&self, ctx: Context, mut member: Member) {
        println!(
            "New member joined: {} ({})",
            member.user.name, member.user.id
        );
        let config = self.store.get_config().await;

        if let Some(welcome_channel) = config.welcome_channel {
            let selection_channel = config.selection_channel.unwrap_or_default();

            // Get guild name for variable substitution
            let guild_name = member
                .guild_id
                .to_guild_cached(&ctx.cache)
                .map(|g| g.name)
                .unwrap_or_else(|| "the server".to_string());

            // Process welcome message with variable substitution
            let processed_message = config
                .welcome_message
                .replace("{user}", &member.mention().to_string())
                .replace("{server}", &guild_name)
                .replace("{channel}", &format!("<#{}>", selection_channel));

            // Send welcome message with embed
            println!("Sending welcome message for new member");
            match ChannelId(welcome_channel)
                .send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title(format!("Welcome to {}!", guild_name))
                            .description(processed_message)
                            .color(0x00_ff_00)
                            .thumbnail(&member.user.face())
                            .footer(|f| f.text("Enjoy your stay!"))
                    })
                })
                .await
            {
                Ok(_) => println!("Sent welcome message successfully"),
                Err(e) => println!("Error sending welcome message: {:?}", e),
            };

            // Auto-assign roles if configured
            if !config.auto_roles.is_empty() {
                for role_id in &config.auto_roles {
                    match member.add_role(&ctx.http, RoleId(*role_id)).await {
                        Ok(_) => println!("Added auto-role {} to new member", role_id),
                        Err(e) => println!("Error adding auto-role: {:?}", e),
                    }
                }
            }
        } else {
            println!("No welcome channel configured, skipping welcome message");
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore bot messages
        if msg.author.bot {
            return;
        }

        // Debug logging
        println!("=== MESSAGE RECEIVED ===");
        println!("Author: {} (ID: {})", msg.author.name, msg.author.id);
        println!("Is DM: {}", msg.guild_id.is_none());
        println!("Content: {}", msg.content);
        println!("Channel ID: {}", msg.channel_id);

        // HANDLE DMs FIRST - This is critical!
        if msg.guild_id.is_none() {
            println!("Processing DM from {}", msg.author.name);
            
            // Let Hikari handle ALL DMs
            let hikari_handled = {
                let mut hikari = self.hikari.write().await;
                hikari.handle_message(ctx.clone(), msg.clone()).await
            };
            
            if hikari_handled {
                println!("Hikari handled the DM successfully");
                return;
            } else {
                println!("Hikari failed to handle DM, sending fallback");
                // Fallback DM response
                let _ = msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title("Hey there! 💜")
                            .description("I'm having a little trouble right now, but I'm still here! Try talking to me again or use !hikarihelp for commands!")
                            .color(0x9B59B6)
                    })
                }).await;
            }
            return;
        }

        // For guild messages, check scanner first
        let scanner_handled = self.scanner.handle_scan_events(ctx.clone(), msg.clone()).await;
        if scanner_handled {
            println!("Scanner handled the message");
            return;
        }
        
        // Then check if Hikari should handle this message
        let hikari_handled = {
            let mut hikari = self.hikari.write().await;
            hikari.handle_message(ctx.clone(), msg.clone()).await
        };
        if hikari_handled {
            println!("Hikari handled the guild message");
            return;
        }

        println!(
            "Processing regular command from {}: {}",
            msg.author.name, msg.content
        );

        // Track message for stats (only for guild messages)
        if let Some(_guild_id) = msg.guild_id {
            self.store.track_message(msg.author.id.0).await;

            let config = self.store.get_config().await;

            // Special case for !config display - allowed anywhere
            if msg.content.starts_with("!config display") {
                println!("Processing display command in any channel");
                self.handle_admin_command(&ctx, &msg).await;
                return;
            }

            // Admin commands check - only process in #waifubot-cmd channel
            if msg.content.starts_with("!config") || msg.content.starts_with("!winner") {
                // Get channel name
                let channel_name = match msg.channel_id.to_channel(&ctx.http).await {
                    Ok(channel) => match channel.guild() {
                        Some(guild_channel) => guild_channel.name.to_lowercase(),
                        None => String::new(),
                    },
                    Err(_) => String::new(),
                };

                // Check if in the admin channel (case-insensitive)
                if channel_name == "waifubot-cmd" {
                    println!("Processing admin command in #waifubot-cmd");

                    // Handle appropriate command
                    if msg.content.starts_with("!config") {
                        self.handle_admin_command(&ctx, &msg).await;
                    } else if msg.content.starts_with("!winner") {
                        self.handle_winner_command(&ctx, &msg).await;
                    }
                } else {
                    // Not in the admin channel - refuse to process
                    println!("Admin command attempted outside of #waifubot-cmd");
                    let _ = msg
                        .channel_id
                        .say(
                            &ctx.http,
                            "Admin commands can only be used in the #waifubot-cmd channel.",
                        )
                        .await;
                }
                return;
            }

            // Handle regular user commands
            if msg.content.starts_with('!') {
                println!("Processing user command");
                self.handle_user_command(&ctx, &msg).await;
            }
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        // Ignore bot reactions
        if let Some(user_id) = reaction.user_id {
            if let Ok(user) = user_id.to_user(&ctx.http).await {
                if user.bot {
                    return;
                }
            }
        }

        if let (Some(guild_id), Some(user_id)) = (reaction.guild_id, reaction.user_id) {
            // Check if this is a custom emoji reaction
            if let ReactionType::Custom { id, .. } = reaction.emoji {
                println!(
                    "Processing reaction: {:?} from user {}",
                    reaction.emoji, user_id
                );

                let config = self.store.get_config().await;

                // Check if this is one of our role emojis
                if let Some(role) = config.roles.iter().find(|r| r.emoji_id == id.0) {
                    println!("Matching role found: {}", role.name);

                    // Add the role to the user
                    match guild_id.member(&ctx.http, user_id).await {
                        Ok(mut member) => {
                            match member.add_role(&ctx.http, RoleId(role.role_id)).await {
                                Ok(_) => {
                                    println!(
                                        "Successfully added role {} to user {}",
                                        role.name, user_id
                                    );

                                    // Update role selection in database
                                    self.store
                                        .update_role_selection(user_id.0, role.emoji_id)
                                        .await;

                                    // Add chat access role if required
                                    if config.require_selection {
                                        if let Some(chat_role) = config.chat_role_id {
                                            if let Err(e) =
                                                member.add_role(&ctx.http, RoleId(chat_role)).await
                                            {
                                                println!("Error adding chat role: {:?}", e);
                                            }
                                        }
                                    }

                                    // Log the role selection if enabled
                                    if let Some(log_channel) = config.log_channel {
                                        let _ = ChannelId(log_channel)
                                            .send_message(&ctx.http, |m| {
                                                m.content(format!(
                                                    "<@{}> selected the role {} ({})",
                                                    user_id.0, role.name, role.emoji_name
                                                ))
                                            })
                                            .await;
                                    }
                                }
                                Err(e) => println!("Error adding role: {:?}", e),
                            }
                        }
                        Err(e) => println!("Error getting guild member: {:?}", e),
                    }
                }
            }
        }
    }

    async fn reaction_remove(&self, ctx: Context, reaction: Reaction) {
        if let (Some(guild_id), Some(user_id)) = (reaction.guild_id, reaction.user_id) {
            // Check if this is a custom emoji reaction
            if let ReactionType::Custom { id, .. } = reaction.emoji {
                println!(
                    "Processing reaction removal: {:?} from user {}",
                    reaction.emoji, user_id
                );

                let config = self.store.get_config().await;

                // Check if this is one of our role emojis
                if let Some(role) = config.roles.iter().find(|r| r.emoji_id == id.0) {
                    println!("Matching role found for removal: {}", role.name);

                    // Remove the role from the user
                    match guild_id.member(&ctx.http, user_id).await {
                        Ok(mut member) => {
                            match member.remove_role(&ctx.http, RoleId(role.role_id)).await {
                                Ok(_) => {
                                    println!(
                                        "Successfully removed role {} from user {}",
                                        role.name, user_id
                                    );

                                    // Update database
                                    self.store.remove_user_role(user_id.0, role.emoji_id).await;

                                    // Log the role removal if enabled
                                    if let Some(log_channel) = config.log_channel {
                                        let _ = ChannelId(log_channel)
                                            .send_message(&ctx.http, |m| {
                                                m.content(format!(
                                                    "<@{}> removed the role {} ({})",
                                                    user_id.0, role.name, role.emoji_name
                                                ))
                                            })
                                            .await;
                                    }
                                }
                                Err(e) => println!("Error removing role: {:?}", e),
                            }
                        }
                        Err(e) => println!("Error getting guild member: {:?}", e),
                    }
                }
            }
        }
    }
}

impl Bot {
    async fn handle_admin_command(&self, ctx: &Context, msg: &Message) {
        println!("Processing admin command: {}", msg.content);

        let args: Vec<&str> = msg.content.split_whitespace().collect();
        match args.get(1).map(|s| *s) {
            Some("welcome") => {
                // Check if this is setting a welcome message or a welcome channel
                if msg.content.contains("<#") && msg.content.contains(">") {
                    // This is setting a welcome channel
                    let channel_id_result = msg
                        .content
                        .split("<#")
                        .nth(1)
                        .and_then(|s| s.split(">").next())
                        .and_then(|id| match id.parse::<u64>() {
                            Ok(parsed_id) => Some(parsed_id),
                            Err(e) => {
                                println!("Error parsing channel ID: {:?}", e);
                                None
                            }
                        });

                    if let Some(channel_id) = channel_id_result {
                        println!("Setting welcome channel to: {}", channel_id);
                        self.store.set_welcome_channel(channel_id).await;
                        let _ = msg.channel_id.say(&ctx.http, "Welcome channel set!").await;
                    } else {
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Invalid channel format. Use #channel-name")
                            .await;
                    }
                } else {
                    // This is setting a welcome message
                    let message = msg.content.splitn(3, ' ').nth(2).unwrap_or_default();
                    self.store.update_welcome_message(message).await;
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Welcome message updated!")
                        .await;
                }
            }
            Some("channel") => {
                // Parse the channel ID from the message
                let channel_id_result = msg
                    .content
                    .split("<#")
                    .nth(1)
                    .and_then(|s| s.split(">").next())
                    .and_then(|id| match id.parse::<u64>() {
                        Ok(parsed_id) => Some(parsed_id),
                        Err(e) => {
                            println!("Error parsing channel ID: {:?}", e);
                            None
                        }
                    });

                if let Some(channel_id) = channel_id_result {
                    println!("Setting selection channel to: {}", channel_id);

                    // Set the selection channel
                    self.store.set_selection_channel(channel_id).await;
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Selection channel set!")
                        .await;

                    // Send selection message
                    let config = self.store.get_config().await;
                    let roles = &config.roles;

                    // Only try to send the selection message if we have roles configured
                    if !roles.is_empty() {
                        println!("Sending selection message with {} roles", roles.len());

                        match ChannelId(channel_id)
                            .send_message(&ctx.http, |m| {
                                m.embed(|e| {
                                    e.title(&config.selection_message.title)
                                        .description(&config.selection_message.description)
                                        .fields(roles.iter().map(|role| {
                                            (
                                                format!(
                                                    "<a:{}:{}> {}",
                                                    role.emoji_name, role.emoji_id, role.name
                                                ),
                                                &role.description,
                                                false,
                                            )
                                        }))
                                        .color(config.selection_message.color);

                                    if let Some(url) = &config.selection_message.image_url {
                                        e.image(url);
                                    }

                                    e
                                })
                            })
                            .await
                        {
                            Ok(selection_msg) => {
                                // Add reactions for each role
                                for role in roles {
                                    println!("Adding reaction for role: {}", role.name);
                                    if let Err(e) = selection_msg
                                        .react(
                                            &ctx.http,
                                            ReactionType::Custom {
                                                animated: true,
                                                id: role.emoji_id.into(),
                                                name: Some(role.emoji_name.clone()),
                                            },
                                        )
                                        .await
                                    {
                                        println!("Error adding reaction: {:?}", e);
                                    }
                                }

                                // Save the message ID for future reference
                                let _ = self.store.set_selection_message(selection_msg.id.0).await;
                            }
                            Err(e) => {
                                println!("Error sending selection message: {:?}", e);
                                let _ = msg
                                    .channel_id
                                    .say(
                                        &ctx.http,
                                        "Failed to send selection message. Check logs for details.",
                                    )
                                    .await;
                            }
                        }
                    } else {
                        let _ = msg.channel_id.say(&ctx.http, "Selection channel set, but no roles are configured yet. Add roles with !config addrole").await;
                    }
                } else {
                    println!("Invalid channel format");
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Invalid channel format. Use #channel-name")
                        .await;
                }
            }
            Some("setimage") => {
                if let Some(url) = args.get(2) {
                    self.store.set_selection_image(url.to_string()).await;
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Selection image updated!")
                        .await;
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide an image URL")
                        .await;
                }
            }
            Some("setdesc") => {
                if args.len() >= 3 {
                    let desc = args[2..].join(" ");
                    self.store.set_selection_description(desc).await;
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Selection description updated!")
                        .await;
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide a description")
                        .await;
                }
            }
            Some("display") => {
                let config = self.store.get_config().await;
                if config.roles.is_empty() {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "No roles configured yet")
                        .await;
                    return;
                }

                println!("Creating selection message display");
                match msg
                    .channel_id
                    .send_message(&ctx.http, |m| {
                        m.embed(|e| {
                            e.title(&config.selection_message.title)
                                .description(&config.selection_message.description)
                                .fields(config.roles.iter().map(|role| {
                                    (
                                        format!(
                                            "<a:{}:{}> {}",
                                            role.emoji_name, role.emoji_id, role.name
                                        ),
                                        &role.description,
                                        false,
                                    )
                                }));

                            if let Some(url) = &config.selection_message.image_url {
                                e.image(url);
                            }

                            e.color(config.selection_message.color)
                        })
                    })
                    .await
                {
                    Ok(message) => {
                        println!("Selection message created successfully");
                        // Add reactions for each role
                        for role in &config.roles {
                            if let Err(e) = message
                                .react(
                                    &ctx.http,
                                    ReactionType::Custom {
                                        animated: true,
                                        id: role.emoji_id.into(),
                                        name: Some(role.emoji_name.clone()),
                                    },
                                )
                                .await
                            {
                                println!("Error adding reaction: {:?}", e);
                            }
                        }

                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Selection message created!")
                            .await;

                        // Save message ID for future reference
                        let _ = self.store.set_selection_message(message.id.0).await;
                    }
                    Err(e) => {
                        println!("Error creating message: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error creating selection message")
                            .await;
                    }
                }
            }
            Some("addrole") => {
                println!(
                    "Processing addrole command with roles: {:?}",
                    msg.mention_roles
                );
                if let Some(role_id) = msg.mention_roles.first() {
                    let args: Vec<&str> = msg.content.split_whitespace().collect();
                    if args.len() >= 6 {
                        let emoji_id = match args[4].parse::<u64>() {
                            Ok(id) => id,
                            Err(_) => {
                                let _ = msg
                                    .channel_id
                                    .say(&ctx.http, "Invalid emoji ID format")
                                    .await;
                                return;
                            }
                        };

                        self.store
                            .add_character_role(
                                args[3],                                  // Name
                                emoji_id,                                 // EmojiID
                                args[5].to_string(),                      // EmojiName
                                role_id.0,                                // RoleID
                                args.get(6..).unwrap_or(&[""]).join(" "), // Description
                            )
                            .await;
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, format!("Character role '{}' added!", args[3]))
                            .await;
                    } else {
                        let _ = msg.channel_id.say(&ctx.http, "Format: !config addrole @role name emoji_id emoji_name description").await;
                    }
                } else {
                    let _ = msg.channel_id.say(&ctx.http, "Role mention required").await;
                }
            }
            Some("removerole") => {
                if let Some(name) = args.get(2) {
                    let config = self.store.get_config().await;
                    if let Some(role) = config
                        .roles
                        .iter()
                        .find(|r| r.name.to_lowercase() == name.to_lowercase())
                    {
                        self.store.remove_character_role(role.emoji_id).await;
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, format!("Removed {} from selection", name))
                            .await;
                    } else {
                        let _ = msg.channel_id.say(&ctx.http, "Role not found").await;
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please specify a role name")
                        .await;
                }
            }
            Some("setrequired") => {
                if let Some(value) = args.get(2) {
                    let require = value.to_lowercase() == "true";
                    let mut config = self.store.get_config().await;
                    config.require_selection = require;
                    drop(config);

                    if let Err(e) = self.store.save_config().await {
                        println!("Error saving setrequired setting: {:?}", e);
                        let _ = msg.channel_id.say(&ctx.http, "Error saving setting").await;
                    } else {
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, format!("Require selection set to: {}", require))
                            .await;
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please specify true or false")
                        .await;
                }
            }
            Some("addmod") => {
                if let Some(role_id) = msg.mention_roles.first() {
                    self.store.set_mod_role(role_id.0).await;
                    let _ = msg.channel_id.say(&ctx.http, "Moderator role added").await;
                } else {
                    let _ = msg.channel_id.say(&ctx.http, "Please mention a role").await;
                }
            }
            Some("addadmin") => {
                if let Some(role_id) = msg.mention_roles.first() {
                    let mut config = self.store.get_config().await;
                    if !config.admin_roles.contains(&role_id.0) {
                        config.admin_roles.push(role_id.0);
                        drop(config);

                        if let Err(e) = self.store.save_config().await {
                            println!("Error saving admin role: {:?}", e);
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "Error saving admin role")
                                .await;
                        } else {
                            let _ = msg.channel_id.say(&ctx.http, "Admin role added").await;
                        }
                    } else {
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "This role is already an admin role")
                            .await;
                    }
                } else {
                    let _ = msg.channel_id.say(&ctx.http, "Please mention a role").await;
                }
            }
            Some("setlogchannel") => {
                let channel_id = msg
                    .content
                    .split("<#")
                    .nth(1)
                    .and_then(|s| s.split(">").next())
                    .and_then(|id| id.parse::<u64>().ok());

                if let Some(channel_id) = channel_id {
                    let mut config = self.store.get_config().await;
                    config.log_channel = Some(channel_id);
                    drop(config);

                    if let Err(e) = self.store.save_config().await {
                        println!("Error saving log channel: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error setting log channel")
                            .await;
                    } else {
                        let _ = msg.channel_id.say(&ctx.http, "Log channel set").await;
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please mention a channel")
                        .await;
                }
            }
            Some("backup") => match self.store.backup_database().await {
                Ok(_) => {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Database backup created successfully")
                        .await;
                }
                Err(e) => {
                    println!("Error creating backup: {:?}", e);
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Error creating database backup")
                        .await;
                }
            },

            // New gacha admin commands
            Some("waifu") => {
                self.handle_waifu_admin_command(ctx, msg).await;
            }
            Some("setchannel") if args.get(2) == Some(&"gacha") => {
                // Parse the channel ID from the message
                let channel_id_result = msg
                    .content
                    .split("<#")
                    .nth(1)
                    .and_then(|s| s.split(">").next())
                    .and_then(|id| id.parse::<u64>().ok());

                if let Some(channel_id) = channel_id_result {
                    match self.store.set_gacha_channel(channel_id).await {
                        Ok(_) => {
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "Gacha channel has been set!")
                                .await;
                        }
                        Err(e) => {
                            println!("Error setting gacha channel: {:?}", e);
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "Error setting gacha channel")
                                .await;
                        }
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Invalid channel format. Use #channel-name")
                        .await;
                }
            }
            Some("freepulls") => {
                if let Some(amount) = args.get(2).and_then(|s| s.parse::<u32>().ok()) {
                    match self.store.get_gacha_config().await {
                        Ok(mut config) => {
                            config.daily_free_pulls = amount;
                            match self.store.save_gacha_config(&config).await {
                                Ok(_) => {
                                    let _ = msg
                                        .channel_id
                                        .say(
                                            &ctx.http,
                                            format!("Daily free pulls set to {}", amount),
                                        )
                                        .await;
                                }
                                Err(e) => {
                                    println!("Error saving gacha config: {:?}", e);
                                    let _ = msg
                                        .channel_id
                                        .say(&ctx.http, "Error setting daily free pulls")
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            println!("Error getting gacha config: {:?}", e);
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "Error accessing gacha configuration")
                                .await;
                        }
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide a valid number")
                        .await;
                }
            }
            Some("giveall") => {
                if let Some(amount) = args.get(3).and_then(|s| s.parse::<u32>().ok()) {
                    if args.get(2) == Some(&"diamonds") {
                        // Get all user profiles
                        let profiles = self.store.get_all_user_profiles().await;
                        let mut success_count = 0;

                        for profile in profiles {
                            match self.store.add_diamonds(profile.user_id, amount).await {
                                Ok(_) => success_count += 1,
                                Err(e) => println!(
                                    "Error giving diamonds to user {}: {:?}",
                                    profile.user_id, e
                                ),
                            }
                        }

                        let _ = msg
                            .channel_id
                            .say(
                                &ctx.http,
                                format!("Gave {} diamonds to {} users", amount, success_count),
                            )
                            .await;
                    } else {
                        let _ = msg
                            .channel_id
                            .say(
                                &ctx.http,
                                "Unknown resource. Use: !config giveall diamonds [amount]",
                            )
                            .await;
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide a valid amount")
                        .await;
                }
            }
            Some("give") => {
                if args.len() >= 5 {
                    if let Some(user_id) = msg.mentions.first().map(|u| u.id.0) {
                        if args[2] == "diamonds" {
                            if let Ok(amount) = args[4].parse::<u32>() {
                                match self.store.add_diamonds(user_id, amount).await {
                                    Ok(new_balance) => {
                                        let _ = msg
                                            .channel_id
                                            .say(
                                                &ctx.http,
                                                format!(
                                                    "Gave {} diamonds to <@{}>. New balance: {}",
                                                    amount, user_id, new_balance
                                                ),
                                            )
                                            .await;
                                    }
                                    Err(e) => {
                                        println!("Error giving diamonds: {:?}", e);
                                        let _ = msg
                                            .channel_id
                                            .say(&ctx.http, "Error processing diamond gift")
                                            .await;
                                    }
                                }
                            } else {
                                let _ =
                                    msg.channel_id.say(&ctx.http, "Invalid amount format").await;
                            }
                        } else {
                            let _ = msg
                                .channel_id
                                .say(
                                    &ctx.http,
                                    "Unknown resource. Use: !config give @user diamonds [amount]",
                                )
                                .await;
                        }
                    } else {
                        let _ = msg.channel_id.say(&ctx.http, "Please mention a user").await;
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Format: !config give @user diamonds [amount]")
                        .await;
                }
            }
            Some("help") => {
                let _ = msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title("Admin Commands")
                            .description("List of available admin commands")
                            .field("!config welcome [message]", "Set welcome message (use {user}, {server}, {channel} for variables)", false)
                            .field("!config welcome #channel", "Set welcome channel", false)
                            .field("!config channel #channel", "Set selection channel and create selection message", false)
                            .field("!config display", "Display and create selection message in current channel", false)
                            .field("!config addrole @role name emoji_id emoji_name description", "Add character role", false)
                            .field("!config removerole [name]", "Remove character role", false)
                            .field("!config setimage [url]", "Set image for selection message", false)
                            .field("!config setdesc [text]", "Set description for selection message", false)
                            .field("!config addmod @role", "Add moderator role", false)
                            .field("!config addadmin @role", "Add admin role", false)
                            .field("!config setlogchannel #channel", "Set log channel", false)
                            .field("!config backup", "Create a database backup", false)
                            // Add new gacha commands
                            .field("!config waifu add [name] [tier] [description]", "Add a new waifu (attach image)", false)
                            .field("!config waifu edit [id] [field] [value]", "Edit a waifu (name/description/tier)", false)
                            .field("!config waifu delete [id]", "Delete a waifu", false)
                            .field("!config waifu list", "List all waifus", false)
                            .field("!config setchannel gacha #channel", "Set the gacha channel", false)
                            .field("!config freepulls [amount]", "Set daily free pulls", false)
                            .field("!config give @user diamonds [amount]", "Give diamonds to user", false)
                            .field("!config giveall diamonds [amount]", "Give diamonds to all users", false)
                            .field("!winner [role_name]", "Award 50 points to users with specified role", false)
                            .color(0x00BFFF)
                    })
                }).await;
            }
            _ => {
                let _ = msg
                    .channel_id
                    .say(
                        &ctx.http,
                        "Unknown command. Use !config help for commands list.",
                    )
                    .await;
            }
        }
    }

    async fn handle_waifu_admin_command(&self, ctx: &Context, msg: &Message) {
        let args: Vec<&str> = msg.content.split_whitespace().collect();
        if args.len() < 3 {
            let _ = msg
                .channel_id
                .say(
                    &ctx.http,
                    "Invalid waifu command. Use !config help for syntax",
                )
                .await;
            return;
        }

        match args[2] {
            "add" => {
                if args.len() < 6 {
                    let _ = msg.channel_id.say(&ctx.http,
                        "Format: !config waifu add [name] [tier] [description...] (with image attachment)").await;
                    return;
                }

                let name = args[3].to_string();
                let tier_str = args[4].to_uppercase();
                let description = args[5..].join(" ");

                // Parse tier
                let tier = match tier_str.as_str() {
                    "A" => WaifuTier::A,
                    "S" => WaifuTier::S,
                    "SS" => WaifuTier::SS,
                    "SSR" => WaifuTier::SSR,
                    _ => {
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Invalid tier. Use A, S, SS, or SSR")
                            .await;
                        return;
                    }
                };

                // Check for image attachment
                if let Some(attachment) = msg.attachments.first() {
                    // Add the waifu
                    match self
                        .store
                        .add_waifu(
                            name.clone(),
                            attachment.url.clone(),
                            description,
                            tier,
                            msg.author.id.0,
                        )
                        .await
                    {
                        Ok(waifu_id) => {
                            let _ = msg
                                .channel_id
                                .say(
                                    &ctx.http,
                                    format!("Added waifu '{}' with ID: {}", name, waifu_id),
                                )
                                .await;
                        }
                        Err(e) => {
                            println!("Error adding waifu: {:?}", e);
                            let _ = msg.channel_id.say(&ctx.http, "Error adding waifu").await;
                        }
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please attach an image for the waifu")
                        .await;
                }
            }
            "edit" => {
                if args.len() < 6 {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Format: !config waifu edit [id] [field] [value]")
                        .await;
                    return;
                }

                if let Ok(waifu_id) = args[3].parse::<u64>() {
                    let field = args[4].to_lowercase();
                    let value = args[5..].join(" ");

                    // Get the waifu
                    match self.store.get_waifu(waifu_id).await {
                        Ok(Some(mut waifu)) => {
                            match field.as_str() {
                                "name" => {
                                    waifu.name = value;
                                }
                                "description" => {
                                    waifu.description = value;
                                }
                                "tier" => {
                                    let tier = match value.to_uppercase().as_str() {
                                        "A" => WaifuTier::A,
                                        "S" => WaifuTier::S,
                                        "SS" => WaifuTier::SS,
                                        "SSR" => WaifuTier::SSR,
                                        _ => {
                                            let _ = msg
                                                .channel_id
                                                .say(
                                                    &ctx.http,
                                                    "Invalid tier. Use A, S, SS, or SSR",
                                                )
                                                .await;
                                            return;
                                        }
                                    };
                                    waifu.base_tier = tier;
                                }
                                "image" => {
                                    if let Some(attachment) = msg.attachments.first() {
                                        waifu.image_url = attachment.url.clone();
                                    } else {
                                        let _ = msg
                                            .channel_id
                                            .say(&ctx.http, "Please attach an image to update")
                                            .await;
                                        return;
                                    }
                                }
                                _ => {
                                    let _ = msg
                                        .channel_id
                                        .say(
                                            &ctx.http,
                                            "Invalid field. Use: name, description, tier, or image",
                                        )
                                        .await;
                                    return;
                                }
                            }

                            // Save updated waifu
                            match self.store.update_waifu(&waifu).await {
                                Ok(_) => {
                                    let _ = msg
                                        .channel_id
                                        .say(
                                            &ctx.http,
                                            format!(
                                                "Updated waifu '{}' (ID: {})",
                                                waifu.name, waifu_id
                                            ),
                                        )
                                        .await;
                                }
                                Err(e) => {
                                    println!("Error updating waifu: {:?}", e);
                                    let _ =
                                        msg.channel_id.say(&ctx.http, "Error updating waifu").await;
                                }
                            }
                        }
                        Ok(None) => {
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, format!("No waifu found with ID: {}", waifu_id))
                                .await;
                        }
                        Err(e) => {
                            println!("Error getting waifu: {:?}", e);
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "Error accessing waifu data")
                                .await;
                        }
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Invalid waifu ID format")
                        .await;
                }
            }
            "delete" => {
                if args.len() < 4 {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Format: !config waifu delete [id]")
                        .await;
                    return;
                }

                if let Ok(waifu_id) = args[3].parse::<u64>() {
                    match self.store.delete_waifu(waifu_id).await {
                        Ok(true) => {
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, format!("Deleted waifu with ID: {}", waifu_id))
                                .await;
                        }
                        Ok(false) => {
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, format!("No waifu found with ID: {}", waifu_id))
                                .await;
                        }
                        Err(e) => {
                            println!("Error deleting waifu: {:?}", e);
                            let _ = msg.channel_id.say(&ctx.http, "Error deleting waifu").await;
                        }
                    }
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Invalid waifu ID format")
                        .await;
                }
            }
            "list" => {
                match self.store.get_all_waifus().await {
                    Ok(waifus) => {
                        if waifus.is_empty() {
                            let _ = msg
                                .channel_id
                                .say(&ctx.http, "No waifus have been added yet")
                                .await;
                            return;
                        }

                        // Create pages of 10 waifus each
                        let chunks = waifus.chunks(10);
                        let total_pages = (waifus.len() + 9) / 10; // Ceiling division

                        for (page_num, chunk) in chunks.enumerate() {
                            let _ = msg
                                .channel_id
                                .send_message(&ctx.http, |m| {
                                    m.embed(|e| {
                                        e.title(format!(
                                            "Waifu List (Page {}/{})",
                                            page_num + 1,
                                            total_pages
                                        ))
                                        .description(
                                            chunk
                                                .iter()
                                                .map(|w| {
                                                    format!(
                                                        "ID: {} | {} | Tier: {} | {}",
                                                        w.id,
                                                        w.name,
                                                        w.base_tier,
                                                        w.description
                                                            .chars()
                                                            .take(30)
                                                            .collect::<String>()
                                                    )
                                                })
                                                .collect::<Vec<_>>()
                                                .join("\n"),
                                        )
                                        .color(0xF8C8DC) // Pink color
                                    })
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        println!("Error getting waifus: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error retrieving waifu list")
                            .await;
                    }
                }
            }
            _ => {
                let _ = msg
                    .channel_id
                    .say(
                        &ctx.http,
                        "Unknown waifu command. Available: add, edit, delete, list",
                    )
                    .await;
            }
        }
    }

    async fn handle_winner_command(&self, ctx: &Context, msg: &Message) {
        // Get the role name from the command
        let args: Vec<&str> = msg.content.split_whitespace().collect();
        if args.len() >= 2 {
            let role_name = args[1];

            println!("Processing winner command for role: {}", role_name);

            // Award points to users with this role
            let awarded_users = self.store.award_points_for_role(role_name, 50).await;

            if !awarded_users.is_empty() {
                println!("Awarded points to {} users", awarded_users.len());

                let _ = msg
                    .channel_id
                    .send_message(&ctx.http, |m| {
                        m.embed(|e| {
                            e.title("🎉 Points Awarded!")
                                .description(format!(
                                    "Awarded 50 points to {} users with the '{}' role!",
                                    awarded_users.len(),
                                    role_name
                                ))
                                .color(0xFFD700)
                        })
                    })
                    .await;
            } else {
                println!("No users found with role: {}", role_name);
                let _ = msg
                    .channel_id
                    .say(
                        &ctx.http,
                        format!("No users found with the '{}' role.", role_name),
                    )
                    .await;
            }
        } else {
            let _ = msg
                .channel_id
                .say(&ctx.http, "Please specify a role name: !winner [role_name]")
                .await;
        }
    }

    async fn handle_user_command(&self, ctx: &Context, msg: &Message) {
        match msg.content.split_whitespace().next().unwrap_or("") {
            // Existing commands
            "!profile" => {
                println!("Processing !profile command for user: {}", msg.author.id.0);

                // Get the user profile
                match self.store.get_user_profile(msg.author.id.0).await {
                    Some(profile) => {
                        let config = self.store.get_config().await;

                        // Get all of user's selected roles
                        let selected_roles: Vec<String> = profile
                            .selected_roles
                            .iter()
                            .filter_map(|emoji_id| {
                                config
                                    .roles
                                    .iter()
                                    .find(|r| &r.emoji_id == emoji_id)
                                    .map(|r| r.name.clone())
                            })
                            .collect();

                        let roles_str = if selected_roles.is_empty() {
                            "None".to_string()
                        } else {
                            selected_roles.join(", ")
                        };

                        // Get user's diamonds with passive income calculation
                        let diamonds = match self.store.get_user_diamonds(msg.author.id.0).await {
                            Ok(diamonds) => diamonds,
                            Err(_) => profile.diamonds, // Fallback to stored value
                        };

                        // Check for user's top waifu
                        let waifu_field = match self.store.get_user_favorite(msg.author.id.0).await
                        {
                            Ok(Some((waifu, user_waifu))) => {
                                let tier = WaifuTier::from_count(user_waifu.count);
                                format!("{} ({})", waifu.name, tier.display_name())
                            }
                            _ => "None".to_string(),
                        };

                        // Send profile embed
                        match msg
                            .channel_id
                            .send_message(&ctx.http, |m| {
                                m.embed(|e| {
                                    e.title(format!("{}'s Profile", msg.author.name))
                                        .thumbnail(msg.author.face())
                                        .field("Character", roles_str, false)
                                        .field("Best Waifu", waifu_field, false)
                                        .field(
                                            "Join Date",
                                            profile.join_date.format("%Y-%m-%d").to_string(),
                                            true,
                                        )
                                        .field("Points", format!("{}", profile.points), true)
                                        .field("Diamonds", format!("💎 {}", diamonds), true)
                                        .field(
                                            "Messages",
                                            format!("{}", profile.messages_sent),
                                            true,
                                        )
                                        .color(0x9B59B6)
                                })
                            })
                            .await
                        {
                            Ok(_) => println!(
                                "Successfully displayed profile for user: {}",
                                msg.author.id.0
                            ),
                            Err(e) => println!("Error displaying profile: {:?}", e),
                        }
                    }
                    None => {
                        // Create a new profile if none exists
                        let new_profile = config::UserProfile {
                            user_id: msg.author.id.0,
                            selected_roles: Vec::new(),
                            join_date: Utc::now(),
                            last_active: Utc::now(),
                            messages_sent: 0,
                            points: 0,
                            diamonds: 0,
                        };

                        // Save the new profile
                        self.store.save_user_profile(&new_profile).await;

                        let _ = msg.channel_id.say(&ctx.http,
                            "You don't have a profile yet. React to the selection message to choose a character!").await;
                    }
                }
            }
            "!rankings" => {
                println!("Processing !rankings command");
                let rankings = self.store.get_rankings().await;

                if rankings.is_empty() {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "No character rankings available yet.")
                        .await;
                    return;
                }

                let _ = msg
                    .channel_id
                    .send_message(&ctx.http, |m| {
                        m.embed(|e| {
                            e.title("Character Rankings")
                                .description(
                                    rankings
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (name, count))| {
                                            format!("{}. {} - {} selections", i + 1, name, count)
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                )
                                .color(0x3498DB)
                        })
                    })
                    .await;
            }
            "!leaderboard" => {
                println!("Processing !leaderboard command");
                let leaderboard = self.store.get_points_leaderboard(10).await;

                if leaderboard.is_empty() {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "No users have earned points yet.")
                        .await;
                    return;
                }

                let mut description = String::new();
                for (idx, (user_id, points)) in leaderboard.iter().enumerate() {
                    // Try to get username
                    let username = match UserId(*user_id).to_user(&ctx.http).await {
                        Ok(user) => user.name,
                        Err(_) => format!("User {}", user_id),
                    };

                    description.push_str(&format!(
                        "{}. **{}** - {} points\n",
                        idx + 1,
                        username,
                        points
                    ));
                }

                match msg
                    .channel_id
                    .send_message(&ctx.http, |m| {
                        m.embed(|e| {
                            e.title("🏆 Points Leaderboard")
                                .description(description)
                                .color(0xF1C40F)
                                .footer(|f| f.text("Use !profile to see your points"))
                        })
                    })
                    .await
                {
                    Ok(_) => println!("Successfully displayed leaderboard"),
                    Err(e) => println!("Error displaying leaderboard: {:?}", e),
                }
            }

            // New gacha commands
            "!diamonds" => match self.store.get_user_diamonds(msg.author.id.0).await {
                Ok(diamonds) => {
                    let _ = msg
                        .channel_id
                        .send_message(&ctx.http, |m| {
                            m.embed(|e| {
                                e.title("Your Diamonds")
                                    .description(format!(
                                        "You currently have 💎 **{}** diamonds",
                                        diamonds
                                    ))
                                    .footer(|f| {
                                        f.text(format!("Pulls cost {} diamonds each", PULL_COST))
                                    })
                                    .color(0x3498DB)
                            })
                        })
                        .await;
                }
                Err(e) => {
                    println!("Error getting diamonds: {:?}", e);
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Error retrieving your diamonds")
                        .await;
                }
            },
            "!pull" => {
                // Check if command is used in the gacha channel
                match self.store.get_gacha_config().await {
                    Ok(gacha_config) => {
                        if let Some(gacha_channel) = gacha_config.gacha_channel {
                            if msg.channel_id.0 != gacha_channel {
                                let _ = msg
                                    .channel_id
                                    .say(
                                        &ctx.http,
                                        format!(
                                            "Please use the gacha commands in <#{}>",
                                            gacha_channel
                                        ),
                                    )
                                    .await;
                                return;
                            }
                        }

                        // Check for daily free pulls
                        let free_pulls = match self.store.check_daily_reset(msg.author.id.0).await {
                            Ok(pulls) => pulls,
                            Err(e) => {
                                println!("Error checking daily pulls: {:?}", e);
                                0 // Default to no free pulls on error
                            }
                        };

                        let use_free_pull = free_pulls > 0;

                        // Attempt to pull a waifu
                        match self.store.pull_waifu(msg.author.id.0, use_free_pull).await {
                            Ok(result) => {
                                // Determine color based on tier
                                let color = if result.tier == WaifuTier::SSR {
                                    get_ssr_color()
                                } else {
                                    Color::from(result.tier.color())
                                };

                                // Create a fancy pull result message
                                let _ = msg
                                    .channel_id
                                    .send_message(&ctx.http, |m| {
                                        m.embed(|e| {
                                            e.title(if result.is_new {
                                                format!("🎉 New Waifu! - {}", result.waifu.name)
                                            } else {
                                                format!("You pulled {}", result.waifu.name)
                                            })
                                            // Remove description and use full-size image instead of thumbnail
                                            .image(&result.waifu.image_url)
                                            .field("Tier", result.tier.display_name(), true)
                                            .field(
                                                "Copies",
                                                format!("{}", result.current_count),
                                                true,
                                            );

                                            if result.tier_up {
                                                e.field(
                                                    "Tier Up!",
                                                    "🔼 You've upgraded this waifu's tier!",
                                                    false,
                                                );
                                            }

                                            // Set the footer based on SSR status
                                            if result.tier == WaifuTier::SSR {
                                                e.footer(|f| {
                                                    f.text("✨ SSR WAIFU! CONGRATULATIONS! ✨")
                                                });
                                            }

                                            // Move diamonds display to the bottom (as the last field)
                                            if use_free_pull {
                                                e.field(
                                                    "Free Pull",
                                                    format!(
                                                        "Daily free pull used! {} remaining",
                                                        free_pulls - 1
                                                    ),
                                                    false,
                                                );
                                            } else {
                                                e.field(
                                                    "Diamonds",
                                                    format!(
                                                        "💎 Spent: {} | Remaining: {}",
                                                        result.diamonds_spent,
                                                        result.diamonds_remaining
                                                    ),
                                                    false,
                                                );
                                            }

                                            e.color(color)
                                        })
                                    })
                                    .await;
                            }
                            Err(e) => {
                                // Handle "not enough diamonds" case
                                if e.to_string().contains("Not enough diamonds") {
                                    let _ = msg.channel_id.send_message(&ctx.http, |m| {
                                        m.embed(|e| {
                                            e.title("Not Enough Diamonds")
                                                .description(format!("You need 💎 **{}** diamonds to pull. Get more by sending messages or waiting!", PULL_COST))
                                                .color(0xE74C3C) // Red
                                        })
                                    }).await;
                                } else {
                                    println!("Error pulling waifu: {:?}", e);
                                    let _ = msg
                                        .channel_id
                                        .say(&ctx.http, "Error processing gacha pull")
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("Error getting gacha config: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error accessing gacha system")
                            .await;
                    }
                }
            }
            "!waifu" => {
                // Display user's favorite/best waifu
                match self.store.get_user_favorite(msg.author.id.0).await {
                    Ok(Some((waifu, user_waifu))) => {
                        let tier = WaifuTier::from_count(user_waifu.count);
                        let color = if tier == WaifuTier::SSR {
                            get_ssr_color()
                        } else {
                            Color::from(tier.color())
                        };

                        let _ = msg
                            .channel_id
                            .send_message(&ctx.http, |m| {
                                m.embed(|e| {
                                    // Use title to show both username and waifu name together
                                    let mut e = e
                                        .title(format!(
                                            "{}'s Top Waifu: {}",
                                            msg.author.name, waifu.name
                                        ))
                                        // Put description at the top of the embed
                                        .description(&waifu.description)
                                        // Use image (not thumbnail) for full-size display
                                        .image(&waifu.image_url)
                                        // Keep the tier and copies info
                                        .field("Tier", tier.display_name(), true)
                                        .field("Copies", format!("{}", user_waifu.count), true)
                                        .color(color);

                                    if tier == WaifuTier::SSR {
                                        e = e.footer(|f| f.text("✨ SSR RARITY ✨"));
                                    }

                                    e
                                })
                            })
                            .await;
                    }
                    Ok(None) => {
                        let _ = msg
                            .channel_id
                            .say(
                                &ctx.http,
                                "You don't have any waifus yet! Use !pull to get your first waifu.",
                            )
                            .await;
                    }
                    Err(e) => {
                        println!("Error getting favorite waifu: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error retrieving your waifu")
                            .await;
                    }
                }
            }
            "!collection" => {
                match self.store.get_user_collection(msg.author.id.0).await {
                    Ok(collection) => {
                        if collection.is_empty() {
                            let _ = msg.channel_id.say(&ctx.http, "You don't have any waifus yet! Use !pull to start your collection.").await;
                            return;
                        }

                        // Get all waifus for this user
                        let mut waifu_list = Vec::new();

                        for (waifu_id, user_waifu) in &collection {
                            match self.store.get_waifu(*waifu_id).await {
                                Ok(Some(waifu)) => {
                                    waifu_list.push((waifu, user_waifu.clone()));
                                }
                                _ => continue,
                            }
                        }

                        // Sort by tier (highest first) then by count
                        waifu_list.sort_by(|a, b| {
                            let a_tier = WaifuTier::from_count(a.1.count);
                            let b_tier = WaifuTier::from_count(b.1.count);

                            b_tier
                                .cmp(&a_tier)
                                .then(b.1.count.cmp(&a.1.count))
                                .then(a.0.name.cmp(&b.0.name))
                        });

                        let total_waifus = waifu_list.len();
                        let display_limit = 4; // Limit to top 4 waifus
                        let displayed_waifus = std::cmp::min(total_waifus, display_limit);

                        // Send header message
                        let header_text = if total_waifus > displayed_waifus {
                            format!(
                                "**{}'s Waifu Collection** - Showing top {} of {} total waifus",
                                msg.author.name, displayed_waifus, total_waifus
                            )
                        } else {
                            format!(
                                "**{}'s Waifu Collection** - Total waifus: {}",
                                msg.author.name, total_waifus
                            )
                        };

                        let _ = msg.channel_id.say(&ctx.http, header_text).await;

                        // Send each waifu in a separate message (limited to top 4)
                        for (index, (waifu, user_waifu)) in
                            waifu_list.iter().take(display_limit).enumerate()
                        {
                            let tier = WaifuTier::from_count(user_waifu.count);
                            let color = if tier == WaifuTier::SSR {
                                get_ssr_color()
                            } else {
                                Color::from(tier.color())
                            };

                            println!("Sending waifu #{}: {}", index + 1, waifu.name);

                            let _ = msg
                                .channel_id
                                .send_message(&ctx.http, |m| {
                                    m.embed(|e| {
                                        e.title(&waifu.name)
                                            .description(format!(
                                                "{} | Copies: {}",
                                                tier.display_name(),
                                                user_waifu.count
                                            ))
                                            .thumbnail(&waifu.image_url)
                                            .color(color)
                                    })
                                })
                                .await;
                        }

                        // Add note if we're not showing all waifus
                        if total_waifus > displayed_waifus {
                            let _ = msg
                                .channel_id
                                .say(
                                    &ctx.http,
                                    "Use `!collection full` to see your entire collection.",
                                )
                                .await;
                        }
                    }
                    Err(e) => {
                        println!("Error getting collection: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error retrieving your collection")
                            .await;
                    }
                }
            }
            "!favorite" => {
                let args: Vec<&str> = msg.content.split_whitespace().collect();

                if args.len() < 2 {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide a waifu name: !favorite [name]")
                        .await;
                    return;
                }

                let search_name = args[1..].join(" ");

                // Find waifus matching the name
                match self.store.find_waifu_by_name(&search_name).await {
                    Ok(matches) => {
                        if matches.is_empty() {
                            let _ = msg
                                .channel_id
                                .say(
                                    &ctx.http,
                                    format!("No waifus found matching '{}'", search_name),
                                )
                                .await;
                            return;
                        }

                        // Get user's collection
                        match self.store.get_user_collection(msg.author.id.0).await {
                            Ok(collection) => {
                                // Find the first match in the user's collection
                                for waifu in matches {
                                    if collection.contains_key(&waifu.id) {
                                        // Set as favorite
                                        match self
                                            .store
                                            .set_favorite_waifu(msg.author.id.0, waifu.id)
                                            .await
                                        {
                                            Ok(true) => {
                                                let _ = msg
                                                    .channel_id
                                                    .say(
                                                        &ctx.http,
                                                        format!(
                                                            "Set **{}** as your favorite waifu!",
                                                            waifu.name
                                                        ),
                                                    )
                                                    .await;
                                                return;
                                            }
                                            Ok(false) => continue,
                                            Err(e) => {
                                                println!("Error setting favorite waifu: {:?}", e);
                                                let _ = msg
                                                    .channel_id
                                                    .say(&ctx.http, "Error setting favorite waifu")
                                                    .await;
                                                return;
                                            }
                                        }
                                    }
                                }

                                let _ = msg
                                    .channel_id
                                    .say(
                                        &ctx.http,
                                        format!(
                                            "You don't own any waifus matching '{}'",
                                            search_name
                                        ),
                                    )
                                    .await;
                            }
                            Err(e) => {
                                println!("Error getting collection: {:?}", e);
                                let _ = msg
                                    .channel_id
                                    .say(&ctx.http, "Error accessing your collection")
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        println!("Error finding waifus: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error searching for waifus")
                            .await;
                    }
                }
            }
            "!discard" => {
                let args: Vec<&str> = msg.content.split_whitespace().collect();

                if args.len() < 2 {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Please provide a waifu name: !discard [name]")
                        .await;
                    return;
                }

                let waifu_name = args[1..].join(" ");

                // Attempt to discard the waifu
                match self.store.discard_waifu(msg.author.id.0, &waifu_name).await {
                    Ok(Some((waifu, remaining_count))) => {
                        let tier = WaifuTier::from_count(remaining_count);

                        if remaining_count > 0 {
                            // Waifu copy was discarded but others remain
                            let _ = msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title(format!("Discarded: {}", waifu.name))
                            .description(format!("You discarded one copy of **{}**.\nYou still have **{}** copies remaining.", 
                                waifu.name, remaining_count))
                            .thumbnail(&waifu.image_url)
                            .field("Current Tier", tier.display_name(), true)
                            .color(Color::from(tier.color()))
                    })
                }).await;
                        } else {
                            // Last copy was discarded
                            let _ = msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title(format!("Discarded: {}", waifu.name))
                            .description(format!("You discarded your last copy of **{}**.\nThis waifu has been removed from your collection.", 
                                waifu.name))
                            .thumbnail(&waifu.image_url)
                            .color(0xE74C3C) // Red
                    })
                }).await;
                        }
                    }
                    Ok(None) => {
                        let _ = msg
                            .channel_id
                            .say(
                                &ctx.http,
                                format!("You don't have any waifus matching '{}'", waifu_name),
                            )
                            .await;
                    }
                    Err(e) => {
                        println!("Error discarding waifu: {:?}", e);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Error discarding waifu")
                            .await;
                    }
                }
            }
            "!daily" => match self.store.check_daily_reset(msg.author.id.0).await {
                Ok(free_pulls) => {
                    let _ = msg.channel_id.send_message(&ctx.http, |m| {
                            m.embed(|e| {
                                e.title("Daily Status")
                                    .description(format!("You have **{}** free pulls remaining today.\nCome back tomorrow for more free pulls and a login bonus!", free_pulls))
                                    .color(0x2ECC71)
                            })
                        }).await;
                }
                Err(e) => {
                    println!("Error checking daily reset: {:?}", e);
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Error checking daily status")
                        .await;
                }
            },
            "!help" => {
                println!("Processing !help command");
                let _ = msg
                    .channel_id
                    .send_message(&ctx.http, |m| {
                        m.embed(|e| {
                            e.title("User Commands")
                                .description("List of available commands")
                                .field("!profile", "View your profile and points", false)
                                .field("!rankings", "View character popularity rankings", false)
                                .field("!leaderboard", "View points leaderboard", false)
                                // Gacha commands
                                .field("!diamonds", "Check your diamond balance", false)
                                .field("!pull", "Pull a random waifu (costs 💎 250)", false)
                                .field("!waifu", "Display your favorite waifu", false)
                                .field("!collection", "Show your entire waifu collection", false)
                                .field("!favorite [name]", "Set a waifu as your favorite", false)
                                .field("!discard [name]", "Discard one copy of a waifu", false)
                                .field("!daily", "Check your daily free pulls", false)
                                .color(0x2ECC71)
                        })
                    })
                    .await;
            }
            _ => {} // Ignore unknown commands
        }
    }
}

// Main function
#[tokio::main]
async fn main() {
    // Load environment variables
    if let Err(e) = dotenv::dotenv() {
        println!("Warning: Failed to load .env file: {:?}", e);
    }

    // Get Discord token
    let token = match std::env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(e) => {
            println!(
                "Error: Discord token not found in environment variables: {:?}",
                e
            );
            println!("Please set the DISCORD_TOKEN environment variable");
            return;
        }
    };

    // Get Gemini API key from environment
    let gemini_api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(key) => key,
        Err(e) => {
            println!("Error: Gemini API key not found in environment variables: {:?}", e);
            println!("Please set the GEMINI_API_KEY environment variable");
            return;
        }
    };

    // Initialize database store
    let store = Arc::new(Store::new().await);
    
    // Start background tasks for the store
    store.start_background_tasks().await;

    // Initialize Hikari with Gemini API and proper configuration
    println!("Initializing Hikari with Gemini API...");
    let hikari_config = HikariConfig {
        command_prefix: "!hikari".to_string(),
        name_trigger: true,
        mention_required: true,
        cooldown_seconds: 300,
        memory_enabled: true,
        system_prompt: "You are Hikari, a Gen-Z crypto enthusiast and influencer...".to_string(),
        enhanced_dm_mode: true,
        relationship_tracking: true,
    };
        
    let mut hikari = Hikari::new(hikari_config, gemini_api_key);

    // Initialize Hikari
    match hikari.init().await {
        Ok(_) => println!("Hikari initialized successfully with DM support"),
        Err(e) => {
            println!("Error initializing Hikari with Gemini API: {:?}", e);
            println!("Check your API key and internet connection");
            return;
        }
    }

    // Create Arc-wrapped Hikari for sharing
    let hikari = Arc::new(RwLock::new(hikari));

    // Initialize scanner
    let scanner = scanner::create_scanner();

    // Build client with ALL required intents including DMs
    println!("Creating Discord client with DM support...");
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::DIRECT_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGE_TYPING;

    let client_builder = Client::builder(&token, intents);

    // Create client with the Bot struct
    let mut client = match client_builder.event_handler(Bot { 
        store,
        hikari,
        scanner: scanner.clone(),
    }).await {
        Ok(client) => client,
        Err(e) => {
            println!("Error creating client: {:?}", e);
            return;
        }
    };

    // Start client
    println!("Starting bot with DM support enabled...");
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}