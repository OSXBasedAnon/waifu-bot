use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::channel::ReactionType;
use serenity::model::gateway::Ready;
use serenity::model::id::{RoleId, UserId, GuildId};
use serenity::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

// Data structures
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WaifuCharacter {
    name: String,
    emoji_id: u64,
    emoji_name: String,
    role_id: u64,
    description: String,
    selection_count: u32,
    last_reset: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UserProfile {
    user_id: u64,
    selected_waifu: u64,  // emoji_id of selected waifu
    team_id: Option<String>,
    join_date: DateTime<Utc>,
    last_selection_date: DateTime<Utc>,
    previous_selections: Vec<u64>,  // history of selections
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Team {
    id: String,
    name: String,
    leader_id: u64,
    members: Vec<u64>,
    created_at: DateTime<Utc>,
}

struct WaifuBot {
    db: sled::Db,
    characters: Arc<RwLock<HashMap<u64, WaifuCharacter>>>,
}

impl WaifuBot {
    async fn new() -> Self {
        let db = sled::open("waifu_bot_data").expect("Failed to open database");
        let characters = Arc::new(RwLock::new(Self::initialize_characters()));
        
        // Initialize character stats from DB
        if let Some(stats) = db.get("character_stats").unwrap() {
            let stored_chars: HashMap<u64, WaifuCharacter> = bincode::deserialize(&stats).unwrap();
            *characters.write().await = stored_chars;
        }

        WaifuBot { db, characters }
    }

    fn initialize_characters() -> HashMap<u64, WaifuCharacter> {
        let mut chars = HashMap::new();
        let character_list = vec![
            ("Maki", 1327355612350382141, "maki"),
            ("Kaguya", 1327356393467940996, "kaguya"),
            ("Marin", 1327356110449016934, "marin"),
            ("Makima", 1327362655521996810, "makima"),
            ("Kurisu", 1327362962624876585, "kurisu"),
            ("Zero Two", 1327362208480362529, "002"),
            ("Misaka", 1327360466120478750, "mikoto"),
            ("Hinata", 1327361136873574573, "hinata"),
            ("Mikasa", 1327359687930150943, "mikasa"),
            ("Rem", 1327361516743430298, "rem"),
            ("Megumin", 1328441290593144905, "Thumpsup_Megumin"),
            ("Nami", 1327360671725129920, "nami"),
            ("Himiko", 1332070942746349659, "himikoexcited"),
        ];

        for (name, emoji_id, emoji_name) in character_list {
            chars.insert(emoji_id, WaifuCharacter {
                name: name.to_string(),
                emoji_id,
                emoji_name: emoji_name.to_string(),
                role_id: 0, // Set actual role IDs
                description: String::new(), // Add descriptions
                selection_count: 0,
                last_reset: Utc::now(),
            });
        }
        chars
    }

    async fn update_character_stats(&self) {
        let chars = self.characters.read().await;
        let encoded = bincode::serialize(&*chars).unwrap();
        self.db.insert("character_stats", encoded).unwrap();
    }

    async fn get_user_profile(&self, user_id: u64) -> Option<UserProfile> {
        self.db.get(format!("user:{}", user_id).as_bytes())
            .unwrap()
            .map(|data| bincode::deserialize(&data).unwrap())
    }

    async fn save_user_profile(&self, profile: &UserProfile) {
        let encoded = bincode::serialize(profile).unwrap();
        self.db.insert(
            format!("user:{}", profile.user_id).as_bytes(),
            encoded
        ).unwrap();
    }

    async fn get_team(&self, team_id: &str) -> Option<Team> {
        self.db.get(format!("team:{}", team_id).as_bytes())
            .unwrap()
            .map(|data| bincode::deserialize(&data).unwrap())
    }

    async fn save_team(&self, team: &Team) {
        let encoded = bincode::serialize(team).unwrap();
        self.db.insert(
            format!("team:{}", team.id).as_bytes(),
            encoded
        ).unwrap();
    }

    async fn update_waifu_selection(&self, user_id: u64, emoji_id: u64) -> Result<(), Box<dyn std::error::Error>> {
        // Update character selection count
        let mut chars = self.characters.write().await;
        if let Some(character) = chars.get_mut(&emoji_id) {
            character.selection_count += 1;
        }
        self.update_character_stats().await;

        // Update user profile
        let mut profile = self.get_user_profile(user_id).await
            .unwrap_or_else(|| UserProfile {
                user_id,
                selected_waifu: emoji_id,
                team_id: None,
                join_date: Utc::now(),
                last_selection_date: Utc::now(),
                previous_selections: Vec::new(),
            });

        profile.previous_selections.push(profile.selected_waifu);
        profile.selected_waifu = emoji_id;
        profile.last_selection_date = Utc::now();

        self.save_user_profile(&profile).await;
        Ok(())
    }

    async fn get_monthly_rankings(&self) -> Vec<(String, u32)> {
        let chars = self.characters.read().await;
        let mut rankings: Vec<_> = chars.values()
            .map(|c| (c.name.clone(), c.selection_count))
            .collect();
        rankings.sort_by(|a, b| b.1.cmp(&a.1));
        rankings
    }

    async fn reset_monthly_stats(&self) {
        let mut chars = self.characters.write().await;
        for char in chars.values_mut() {
            char.selection_count = 0;
            char.last_reset = Utc::now();
        }
        self.update_character_stats().await;
    }
}

struct Handler {
    bot: Arc<WaifuBot>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        match msg.content.as_str() {
            "!profile" => {
                if let Some(profile) = self.bot.get_user_profile(msg.author.id.0).await {
                    let chars = self.bot.characters.read().await;
                    if let Some(character) = chars.get(&profile.selected_waifu) {
                        msg.channel_id.send_message(&ctx.http, |m| {
                            m.embed(|e| {
                                e.title(format!("{}'s Profile", msg.author.name))
                                    .field("Current Waifu", &character.name, true)
                                    .field("Team", profile.team_id.as_deref().unwrap_or("None"), true)
                                    .field("Join Date", 
                                         profile.join_date.format("%Y-%m-%d").to_string(), 
                                         true)
                            })
                        }).await.unwrap();
                    }
                }
            }
            "!rankings" => {
                let rankings = self.bot.get_monthly_rankings().await;
                msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title("Current Waifu Rankings")
                            .description(rankings.iter()
                                .enumerate()
                                .map(|(i, (name, count))| 
                                    format!("{}. {} - {} selections", i + 1, name, count))
                                .collect::<Vec<_>>()
                                .join("\n"))
                    })
                }).await.unwrap();
            }
            _ => {}
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: serenity::model::channel::Reaction) {
        if let Some(guild_id) = reaction.guild_id {
            if let Some(user_id) = reaction.user_id {
                if let ReactionType::Custom { id, .. } = reaction.emoji {
                    if let Ok(mut member) = guild_id.member(&ctx.http, user_id).await {
                        let emoji_id = id.0;
                        if let Err(e) = self.bot.update_waifu_selection(user_id.0, emoji_id).await {
                            println!("Error updating selection: {:?}", e);
                        }

                        // Update roles
                        let chars = self.bot.characters.read().await;
                        for character in chars.values() {
                            let _ = member.remove_role(&ctx.http, RoleId(character.role_id)).await;
                        }
                        if let Some(character) = chars.get(&emoji_id) {
                            let _ = member.add_role(&ctx.http, RoleId(character.role_id)).await;
                        }
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("DISCORD_TOKEN").expect("Discord token not found");
    
    let bot = Arc::new(WaifuBot::new().await);
    
    let mut client = Client::builder(&token)
        .event_handler(Handler { bot: bot.clone() })
        .await
        .expect("Error creating client");

    // Set up monthly stats reset
    let bot_clone = bot.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await; // Check daily
            let now = Utc::now();
            let chars = bot_clone.characters.read().await;
            if let Some(first_char) = chars.values().next() {
                if (now - first_char.last_reset).num_days() >= 30 {
                    bot_clone.reset_monthly_stats().await;
                }
            }
        }
    });

    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}