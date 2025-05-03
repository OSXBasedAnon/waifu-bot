use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// Custom error type for better error handling
#[derive(Debug)]
pub enum BotError {
    DatabaseError(String),
    SerializationError(String),
    DeserializationError(String),
    IoError(std::io::Error),
}

impl fmt::Display for BotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BotError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            BotError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            BotError::DeserializationError(msg) => write!(f, "Deserialization error: {}", msg),
            BotError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for BotError {}

impl From<std::io::Error> for BotError {
    fn from(error: std::io::Error) -> Self {
        BotError::IoError(error)
    }
}

impl From<bincode::Error> for BotError {
    fn from(error: bincode::Error) -> Self {
        BotError::DeserializationError(error.to_string())
    }
}

impl From<sled::Error> for BotError {
    fn from(error: sled::Error) -> Self {
        BotError::DatabaseError(error.to_string())
    }
}

// Main bot configuration struct
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub admin_guild_id: u64,
    pub welcome_message: String,
    pub welcome_channel: Option<u64>,
    pub selection_message: SelectionMessage,
    pub selection_channel: Option<u64>,
    pub roles: Vec<CharacterRole>,
    pub require_selection: bool,
    pub chat_role_id: Option<u64>,
    pub mod_roles: Vec<u64>,
    pub admin_roles: Vec<u64>,
    pub log_channel: Option<u64>,
    pub auto_roles: Vec<u64>,
    pub verification: VerificationSettings,
    pub monthly_rotation: bool,
    pub stats_tracking: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionMessage {
    pub title: String,
    pub description: String,
    pub image_url: Option<String>,
    pub color: u32, // Hex color code
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationSettings {
    pub enabled: bool,
    pub required_role: Option<u64>,
    pub min_account_age: Option<u32>,
    pub verification_channel: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterRole {
    pub name: String,
    pub emoji_id: u64,
    pub emoji_name: String,
    pub role_id: u64,
    pub description: String,
    pub selection_count: u32,
    pub last_reset: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: u64,
    pub selected_roles: Vec<u64>,
    pub join_date: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub messages_sent: u32,
    pub points: u32,
    pub diamonds: u32,
}

// New struct for optimized Store with caching and batch operations
pub struct Store {
    db: sled::Db,
    config: Arc<RwLock<BotConfig>>,
    // New fields for caching and batch processing
    profile_cache: Arc<RwLock<HashMap<u64, (UserProfile, Instant)>>>,
    pending_profile_saves: Arc<RwLock<HashMap<u64, UserProfile>>>,
    last_batch_save: Arc<RwLock<Instant>>,
    cache_ttl: Duration,
    batch_save_interval: Duration,
    flush_interval: Duration,
}

impl Store {
    pub async fn new() -> Self {
        let db = match sled::open("waifu_bot_data") {
            Ok(db) => db,
            Err(e) => {
                println!("Failed to open database: {:?}. Creating a new one.", e);
                sled::Config::new()
                    .path("waifu_bot_data_new")
                    .create_new(true)
                    .open()
                    .expect("Failed to create new database")
            }
        };

        // Attempt to load the config, falling back to default if needed
        let config = if let Ok(Some(data)) = db.get("config") {
            match bincode::deserialize(&data) {
                Ok(config) => config,
                Err(e) => {
                    println!("Error deserializing config: {:?}. Using default.", e);
                    Self::default_config()
                }
            }
        } else {
            println!("No config found in database. Using default.");
            Self::default_config()
        };

        Store {
            db,
            config: Arc::new(RwLock::new(config)),
            profile_cache: Arc::new(RwLock::new(HashMap::new())),
            pending_profile_saves: Arc::new(RwLock::new(HashMap::new())),
            last_batch_save: Arc::new(RwLock::new(Instant::now())),
            cache_ttl: Duration::from_secs(300),         // 5 minute cache lifetime
            batch_save_interval: Duration::from_secs(5), // Process batch every 5 seconds
            flush_interval: Duration::from_secs(60),     // Flush DB every minute
        }
    }

    // Start background tasks for batch processing and cache management
    pub async fn start_background_tasks(self: &Arc<Self>) {
        let store_clone = Arc::clone(self);
        
        // Spawn background task for batch processing of profile saves
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(store_clone.batch_save_interval);
            let mut flush_counter = 0;
            
            loop {
                interval.tick().await;
                
                // Process any pending saves
                store_clone.process_pending_saves().await;
                
                // Increment counter and check if we should flush
                flush_counter += 1;
                if flush_counter >= (store_clone.flush_interval.as_secs() / store_clone.batch_save_interval.as_secs()) {
                    flush_counter = 0;
                    if let Err(e) = store_clone.db.flush_async().await {
                        println!("Error flushing database: {:?}", e);
                    } else {
                        println!("Database flushed successfully");
                    }
                }
            }
        });
        
        // Spawn background task for cache cleanup
        let store_clone = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Check every minute
            
            loop {
                interval.tick().await;
                
                // Clean expired cache entries
                let now = Instant::now();
                let mut cache = store_clone.profile_cache.write().await;
                let before_count = cache.len();
                
                cache.retain(|_, (_, timestamp)| {
                    timestamp.elapsed() < store_clone.cache_ttl
                });
                
                let removed = before_count - cache.len();
                if removed > 0 {
                    println!("Cleaned {} expired cache entries", removed);
                }
            }
        });
    }

    fn default_config() -> BotConfig {
        BotConfig {
            admin_guild_id: std::env::var("ADMIN_GUILD_ID")
                .unwrap_or_else(|_| "0".to_string())
                .parse()
                .unwrap_or(0),
            welcome_message:
                "Welcome {user} to {server}! Please select your character in {channel}.".to_string(),
            welcome_channel: None,
            selection_channel: None,
            selection_message: SelectionMessage {
                title: "Select Best Girl".to_string(),
                description: "More than one **Waifu** will destroy your **Laifu**".to_string(),
                image_url: None,
                color: 0xFF69B4,
            },
            roles: Vec::new(),
            require_selection: true,
            chat_role_id: None,
            mod_roles: Vec::new(),
            admin_roles: Vec::new(),
            log_channel: None,
            auto_roles: Vec::new(),
            verification: VerificationSettings {
                enabled: false,
                required_role: None,
                min_account_age: None,
                verification_channel: None,
            },
            monthly_rotation: true,
            stats_tracking: true,
        }
    }

    pub async fn get_config(&self) -> BotConfig {
        self.config.read().await.clone()
    }

    // Get data from database with optional caching
    pub async fn db_get(&self, key: &str) -> Result<Option<Vec<u8>>, sled::Error> {
        self.db
            .get(key.as_bytes())
            .map(|opt| opt.map(|ivec| ivec.to_vec()))
    }

    // Insert data into database
    pub async fn db_insert(&self, key: &str, value: &[u8]) -> Result<Option<Vec<u8>>, sled::Error> {
        self.db
            .insert(key.as_bytes(), value)
            .map(|opt| opt.map(|ivec| ivec.to_vec()))
    }

    // Remove data from database
    pub async fn db_remove(&self, key: &str) -> Result<Option<Vec<u8>>, sled::Error> {
        self.db
            .remove(key.as_bytes())
            .map(|opt| opt.map(|ivec| ivec.to_vec()))
    }

    // Flush data to disk
    pub async fn db_flush(&self) -> Result<(), sled::Error> {
        self.db.flush_async().await.map(|_| ())
    }

    // Robust save_config that handles errors
    pub async fn save_config(&self) -> Result<(), BotError> {
        let config = self.config.read().await;

        // Serialize the config
        let serialized = bincode::serialize(&*config)
            .map_err(|e| BotError::SerializationError(e.to_string()))?;

        // Save to database
        self.db
            .insert("config", serialized)
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        // Flush to disk
        self.db
            .flush_async()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub fn create_new_profile(user_id: u64) -> UserProfile {
        UserProfile {
            user_id,
            selected_roles: Vec::new(),
            join_date: Utc::now(),
            last_active: Utc::now(),
            messages_sent: 0,
            points: 0,
            diamonds: 0,
        }
    }

    // Legacy wrapper for backward compatibility
    pub async fn save_config_legacy(&self) {
        if let Err(e) = self.save_config().await {
            println!("Error saving config: {:?}", e);
        }
    }

    // OPTIMIZED: Get user profile with caching
    pub async fn get_user_profile(&self, user_id: u64) -> Option<UserProfile> {
        // Check cache first
        {
            let cache = self.profile_cache.read().await;
            if let Some((profile, timestamp)) = cache.get(&user_id) {
                if timestamp.elapsed() < self.cache_ttl {
                    println!("Cache hit for user: {}", user_id);
                    return Some(profile.clone());
                }
            }
        }

        println!("Retrieving profile for user: {}", user_id);

        // Cache miss, get from database
        let key = format!("user:{}", user_id);
        let result = match self.db.get(key.as_bytes()) {
            Ok(data) => data,
            Err(e) => {
                println!(
                    "Database error when getting profile for user {}: {:?}",
                    user_id, e
                );
                return None;
            }
        };

        // If no data exists, return None
        let data = match result {
            Some(data) => data,
            None => {
                println!("No profile found for user: {}", user_id);
                return None;
            }
        };

        // Try to deserialize the profile
match bincode::deserialize::<UserProfile>(&data) {
    Ok(profile) => {
        println!("Successfully retrieved profile for user: {}", user_id);
        
        // Cache the result
        let mut cache = self.profile_cache.write().await;
        cache.insert(user_id, (profile.clone(), Instant::now()));
        
        Some(profile)
    }
            Err(e) => {
                println!("Error deserializing profile for user {}: {:?}", user_id, e);

                // If profile is corrupted, create a new one
                let new_profile = Self::create_new_profile(user_id);
                println!("Creating new profile for user: {}", user_id);
                
                Some(new_profile)
            }
        }
    }

    // OPTIMIZED: Save user profile with batch processing
    pub async fn save_user_profile(&self, profile: &UserProfile) {
        // Update cache
        {
            let mut cache = self.profile_cache.write().await;
            cache.insert(profile.user_id, (profile.clone(), Instant::now()));
        }
        
        // Queue for batch saving
        {
            let mut pending = self.pending_profile_saves.write().await;
            pending.insert(profile.user_id, profile.clone());
        }
        
        // Check if we should trigger immediate batch processing
        let should_process = {
            let last_save = self.last_batch_save.read().await;
            last_save.elapsed() >= self.batch_save_interval
        };
        
        if should_process {
            self.process_pending_saves().await;
        }
    }
    
    // Process all pending profile saves
    async fn process_pending_saves(&self) {
        // Extract profiles to save
        let profiles_to_save = {
            let mut pending = self.pending_profile_saves.write().await;
            if pending.is_empty() {
                return; // Nothing to process
            }
            
            // Update last save timestamp
            let mut last_save = self.last_batch_save.write().await;
            *last_save = Instant::now();
            
            // Extract values and clear pending
            let profiles = pending.values().cloned().collect::<Vec<_>>();
            pending.clear();
            
            profiles
        };
        
        if !profiles_to_save.is_empty() {
            println!("Batch saving {} user profiles", profiles_to_save.len());
        }
        
        // Save each profile to the database
        for profile in profiles_to_save {
            self.save_profile_to_db(&profile).await;
        }
    }
    
    // Core database save logic without caching
    async fn save_profile_to_db(&self, profile: &UserProfile) {
        // Serialize the profile
        let serialized = match bincode::serialize(profile) {
            Ok(data) => data,
            Err(e) => {
                println!("Error serializing profile for user {}: {:?}", profile.user_id, e);
                return;
            }
        };

        // Save to database
        let key = format!("user:{}", profile.user_id);
        if let Err(e) = self.db.insert(key.as_bytes(), serialized) {
            println!("Error saving profile for user {}: {:?}", profile.user_id, e);
            return;
        }

        println!("Successfully saved profile for user: {}", profile.user_id);
    }

    // OPTIMIZED: Track message with combined operations
    pub async fn track_message(&self, user_id: u64) {
        // Check if stats tracking is enabled
        let config = self.config.read().await;
        if !config.stats_tracking {
            return;
        }
        drop(config);

        // Get or create user profile and update in one operation
        match self.get_user_profile(user_id).await {
            Some(mut profile) => {
                // Update all fields in memory
                profile.messages_sent += 1;
                profile.last_active = Utc::now();
                
                // Save updated profile (will be queued for batch saving)
                self.save_user_profile(&profile).await;
                
                // Add diamonds (handled in waifu.rs)
                if let Err(e) = self.track_message_diamonds(user_id).await {
                    println!("Error tracking message diamonds: {:?}", e);
                }
            }
            None => {
                // Create new profile for new user
                let profile = Self::create_new_profile(user_id);
                self.save_user_profile(&profile).await;

                // Add initial diamonds for first message
                if let Err(e) = self.track_message_diamonds(user_id).await {
                    println!("Error tracking initial message diamonds: {:?}", e);
                }
            }
        }
    }

    pub async fn update_welcome_message(&self, message: &str) {
        println!("Updating welcome message to: {}", message);
        let mut config = self.config.write().await;
        config.welcome_message = message.to_string();
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving welcome message: {:?}", e);
        } else {
            println!("Successfully saved welcome message");
        }
    }

    pub async fn set_welcome_channel(&self, channel_id: u64) {
        println!("Setting welcome channel in store: {}", channel_id);
        let mut config = self.config.write().await;
        config.welcome_channel = Some(channel_id);
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving welcome channel: {:?}", e);
        } else {
            println!("Successfully saved welcome channel to store");
        }
    }

    pub async fn set_selection_channel(&self, channel_id: u64) {
        println!("Setting selection channel: {}", channel_id);
        let mut config = self.config.write().await;
        config.selection_channel = Some(channel_id);
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving selection channel: {:?}", e);
        } else {
            println!("Successfully saved selection channel");
        }
    }

    pub async fn set_selection_message(&self, message_id: u64) -> Result<(), BotError> {
        println!("Setting selection message to ID: {}", message_id);
        let serialized = bincode::serialize(&message_id)
            .map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db
            .insert("selection_message_id", serialized)
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        println!("Successfully saved selection message ID");
        Ok(())
    }

    // Legacy wrapper for backward compatibility
    pub async fn set_selection_message_legacy(&self, message_id: u64) {
        if let Err(e) = self.set_selection_message(message_id).await {
            println!("Error saving selection message ID: {:?}", e);
        }
    }

    pub async fn get_selection_message(&self) -> Option<u64> {
        match self.db.get("selection_message_id") {
            Ok(Some(data)) => match bincode::deserialize(&data) {
                Ok(message_id) => Some(message_id),
                Err(e) => {
                    println!("Error deserializing selection message ID: {:?}", e);
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                println!("Error retrieving selection message ID: {:?}", e);
                None
            }
        }
    }

    pub async fn set_selection_image(&self, url: String) {
        println!("Setting selection image to: {}", url);
        let mut config = self.config.write().await;
        config.selection_message.image_url = Some(url);
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving selection image: {:?}", e);
        } else {
            println!("Successfully saved selection image");
        }
    }

    pub async fn set_selection_description(&self, desc: String) {
        println!("Setting selection description to: {}", desc);
        let mut config = self.config.write().await;
        config.selection_message.description = desc;
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving selection description: {:?}", e);
        } else {
            println!("Successfully saved selection description");
        }
    }

    pub async fn add_character_role(
        &self,
        name: &str,
        emoji_id: u64,
        emoji_name: String,
        role_id: u64,
        description: String,
    ) {
        println!(
            "Adding character role: {} with emoji_id: {}",
            name, emoji_id
        );
        let mut config = self.config.write().await;

        // Check if role already exists to avoid duplicates
        if !config
            .roles
            .iter()
            .any(|r| r.emoji_id == emoji_id || r.role_id == role_id)
        {
            config.roles.push(CharacterRole {
                name: name.to_string(),
                emoji_id,
                emoji_name,
                role_id,
                description,
                selection_count: 0,
                last_reset: Utc::now(),
            });
            drop(config);

            if let Err(e) = self.save_config().await {
                println!("Error saving new character role: {:?}", e);
            } else {
                println!("Successfully added character role: {}", name);
            }
        } else {
            println!("Role already exists, not adding duplicate.");
        }
    }

    pub async fn remove_character_role(&self, emoji_id: u64) {
        println!("Removing character role with emoji_id: {}", emoji_id);
        let mut config = self.config.write().await;

        // Find the role name before removal for logging
        let role_name = config
            .roles
            .iter()
            .find(|r| r.emoji_id == emoji_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        // Remove the role from config
        config.roles.retain(|r| r.emoji_id != emoji_id);
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving after role removal: {:?}", e);
            return;
        }

        // Now clean up user profiles that had this role
        println!("Cleaning up user profiles with role: {}", role_name);
        let profiles = self.get_all_user_profiles().await;

        for mut profile in profiles {
            if profile.selected_roles.contains(&emoji_id) {
                println!("Removing role from user: {}", profile.user_id);
                profile.selected_roles.retain(|&r| r != emoji_id);
                self.save_user_profile(&profile).await;
            }
        }

        println!("Successfully removed character role: {}", role_name);
    }

    pub async fn remove_user_role(&self, user_id: u64, emoji_id: u64) {
        println!(
            "Removing role with emoji_id: {} from user: {}",
            emoji_id, user_id
        );
        if let Some(mut profile) = self.get_user_profile(user_id).await {
            if profile.selected_roles.contains(&emoji_id) {
                profile.selected_roles.retain(|&r| r != emoji_id);
                self.save_user_profile(&profile).await;
                println!("Successfully removed role from user");
            } else {
                println!("User doesn't have this role");
            }
        } else {
            println!("User profile not found");
        }
    }

    pub async fn update_role_selection(&self, user_id: u64, emoji_id: u64) {
        println!(
            "Updating role selection for user: {} with emoji_id: {}",
            user_id, emoji_id
        );

        // Get or create user profile
        let mut profile = self
            .get_user_profile(user_id)
            .await
            .unwrap_or_else(|| UserProfile {
                user_id,
                selected_roles: Vec::new(),
                join_date: Utc::now(),
                last_active: Utc::now(),
                messages_sent: 0,
                points: 0,
                diamonds: 0,
            });

        // Add role if not already selected
        if !profile.selected_roles.contains(&emoji_id) {
            println!("Adding new role to user's profile");
            profile.selected_roles.push(emoji_id);
            profile.last_active = Utc::now();
            self.save_user_profile(&profile).await;

            // Update role selection count in config
            let mut config = self.config.write().await;
            if let Some(role) = config.roles.iter_mut().find(|r| r.emoji_id == emoji_id) {
                role.selection_count += 1;
                println!("Incremented selection count for role: {}", role.name);
            }
            drop(config);

            if let Err(e) = self.save_config().await {
                println!("Error saving updated selection count: {:?}", e);
            }
        } else {
            println!("User already has this role");
        }
    }

    pub async fn get_rankings(&self) -> Vec<(String, u32)> {
        println!("Getting character rankings");
        let config = self.config.read().await;
        let mut rankings: Vec<_> = config
            .roles
            .iter()
            .map(|r| (r.name.clone(), r.selection_count))
            .collect();
        rankings.sort_by(|a, b| b.1.cmp(&a.1));
        println!("Retrieved {} roles for ranking", rankings.len());
        rankings
    }

    pub async fn reset_monthly_stats(&self) {
        println!("Resetting monthly statistics");
        let mut config = self.config.write().await;

        if config.monthly_rotation {
            for role in &mut config.roles {
                role.selection_count = 0;
                role.last_reset = Utc::now();
                println!("Reset stats for role: {}", role.name);
            }
            drop(config);

            if let Err(e) = self.save_config().await {
                println!("Error saving after stats reset: {:?}", e);
            } else {
                println!("Successfully reset all monthly stats");
            }
        } else {
            println!("Monthly rotation is disabled, not resetting stats");
        }
    }

    // OPTIMIZED: Get all user profiles with batch loading
    pub async fn get_all_user_profiles(&self) -> Vec<UserProfile> {
        println!("Retrieving all user profiles");
        let mut profiles = Vec::new();
        
        // Get list of keys first to avoid holding read lock during processing
        let prefix = b"user:";
        let keys = match self.db.scan_prefix(prefix).keys().collect::<Result<Vec<_>, _>>() {
            Ok(keys) => keys,
            Err(e) => {
                println!("Error scanning user profile keys: {:?}", e);
                return Vec::new();
            }
        };
        
        println!("Found {} user profile keys", keys.len());
        
        // Process keys in batches of 50
        const BATCH_SIZE: usize = 50;
        for chunk in keys.chunks(BATCH_SIZE) {
            let mut batch_profiles = Vec::with_capacity(chunk.len());
            
            for key in chunk {
                let key_str = String::from_utf8_lossy(key);
                let user_id_str = key_str.strip_prefix("user:").unwrap_or("");
                
                if let Ok(user_id) = user_id_str.parse::<u64>() {
                    // Check cache first
                    let cache_hit = {
                        let cache = self.profile_cache.read().await;
                        if let Some((profile, timestamp)) = cache.get(&user_id) {
                            if timestamp.elapsed() < self.cache_ttl {
                                batch_profiles.push(profile.clone());
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };
                    
                    if !cache_hit {
                        // Get from database
                        if let Ok(Some(data)) = self.db.get(key) {
                            match bincode::deserialize::<UserProfile>(&data) {
                                Ok(profile) => {
                                    // Update cache
                                    {
                                        let mut cache = self.profile_cache.write().await;
                                        cache.insert(user_id, (profile.clone(), Instant::now()));
                                    }
                                    batch_profiles.push(profile);
                                }
                                Err(e) => {
                                    println!("Error deserializing profile for user {}: {:?}", user_id, e);
                                }
                            }
                        }
                    }
                }
            }
            
            profiles.extend(batch_profiles);
        }

        println!("Retrieved {} user profiles", profiles.len());
        profiles
    }

    pub async fn clean_inactive_roles(&self, days: i64) {
        println!("Cleaning inactive roles older than {} days", days);
        let now = Utc::now();
        let profiles = self.get_all_user_profiles().await;
        let mut cleaned_count = 0;

        for profile in profiles {
            if (now - profile.last_active).num_days() > days {
                let mut updated = profile.clone();

                if !updated.selected_roles.is_empty() {
                    updated.selected_roles.clear();
                    self.save_user_profile(&updated).await;
                    cleaned_count += 1;
                }
            }
        }

        println!("Cleaned roles from {} inactive users", cleaned_count);
    }

    pub async fn set_mod_role(&self, role_id: u64) {
        println!("Adding moderator role: {}", role_id);
        let mut config = self.config.write().await;

        if !config.mod_roles.contains(&role_id) {
            config.mod_roles.push(role_id);
            drop(config);

            if let Err(e) = self.save_config().await {
                println!("Error saving moderator role: {:?}", e);
            } else {
                println!("Successfully added moderator role");
            }
        } else {
            println!("Moderator role already exists");
        }
    }

    pub async fn remove_mod_role(&self, role_id: u64) {
        println!("Removing moderator role: {}", role_id);
        let mut config = self.config.write().await;

        if config.mod_roles.contains(&role_id) {
            config.mod_roles.retain(|&r| r != role_id);
            drop(config);

            if let Err(e) = self.save_config().await {
                println!("Error saving after mod role removal: {:?}", e);
            } else {
                println!("Successfully removed moderator role");
            }
        } else {
            println!("Moderator role not found");
        }
    }

    pub async fn set_verification(&self, settings: VerificationSettings) {
        println!("Updating verification settings");
        let mut config = self.config.write().await;
        config.verification = settings;
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving verification settings: {:?}", e);
        } else {
            println!("Successfully updated verification settings");
        }
    }

    pub async fn check_mod_permissions(&self, user_roles: &[u64]) -> bool {
        let config = self.config.read().await;
        let has_perm = user_roles
            .iter()
            .any(|r| config.mod_roles.contains(r) || config.admin_roles.contains(r));

        if has_perm {
            println!("User has moderator permissions");
        } else {
            println!("User does not have moderator permissions");
        }

        has_perm
    }

    pub async fn toggle_monthly_rotation(&self, enabled: bool) {
        println!("Setting monthly rotation to: {}", enabled);
        let mut config = self.config.write().await;
        config.monthly_rotation = enabled;
        drop(config);

        if let Err(e) = self.save_config().await {
            println!("Error saving monthly rotation setting: {:?}", e);
        } else {
            println!("Successfully updated monthly rotation setting");
        }
    }

    pub async fn award_points_for_role(&self, role_name: &str, points_to_award: u32) -> Vec<u64> {
        println!(
            "Awarding {} points to users with role: {}",
            points_to_award, role_name
        );
        let mut awarded_users = Vec::new();

        // Find the role by name or emoji name
        let config = self.config.read().await;
        let role = config.roles.iter().find(|r| {
            r.name.to_lowercase() == role_name.to_lowercase()
                || r.emoji_name.to_lowercase() == role_name.to_lowercase()
        });

        match role {
            Some(role) => {
                println!("Found role: {} with emoji_id: {}", role.name, role.emoji_id);
                let emoji_id = role.emoji_id;
                drop(config);

                // Get all user profiles
                let profiles = self.get_all_user_profiles().await;
                println!("Checking {} user profiles for role matches", profiles.len());

                // Process each profile
                for mut profile in profiles {
                    if profile.selected_roles.contains(&emoji_id) {
                        println!("Awarding points to user: {}", profile.user_id);
                        profile.points += points_to_award;
                        self.save_user_profile(&profile).await;
                        awarded_users.push(profile.user_id);
                    }
                }

                println!("Awarded points to {} users", awarded_users.len());
            }
            None => {
                println!("Role '{}' not found", role_name);
            }
        }

        awarded_users
    }

    pub async fn get_user_points(&self, user_id: u64) -> u32 {
        if let Some(profile) = self.get_user_profile(user_id).await {
            profile.points
        } else {
            0
        }
    }

    pub async fn get_points_leaderboard(&self, limit: usize) -> Vec<(u64, u32)> {
        println!("Getting points leaderboard (top {})", limit);
        let profiles = self.get_all_user_profiles().await;

        let mut leaderboard: Vec<(u64, u32)> =
            profiles.iter().map(|p| (p.user_id, p.points)).collect();

        // Sort by points (descending)
        leaderboard.sort_by(|a, b| b.1.cmp(&a.1));

        // Return only the top entries
        leaderboard.truncate(limit);
        println!("Retrieved {} users for leaderboard", leaderboard.len());
        leaderboard
    }

    pub async fn backup_database(&self) -> Result<(), BotError> {
        println!("Flushing database to ensure data integrity");

        // Process any pending saves first
        self.process_pending_saves().await;

        // Flush current data to disk
        self.db.flush_async().await?;

        // Create a timestamp for the backup marker
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let backup_info = format!("Database backup created at {}", timestamp);

        // Create a backup info file
        let backup_file = format!("waifu_bot_backup_{}.txt", timestamp);

        match std::fs::write(&backup_file, backup_info.as_bytes()) {
            Ok(_) => {
                println!(
                    "Database flushed and backup marker created: {}",
                    backup_file
                );

                // To provide some actual useful information, also save the bot config
                let config = self.config.read().await;
                if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                    let config_backup = format!("waifu_bot_config_{}.json", timestamp);
                    if let Err(e) = std::fs::write(&config_backup, config_json) {
                        println!("Warning: Could not save config backup: {:?}", e);
                    } else {
                        println!("Config backup saved to: {}", config_backup);
                    }
                }

                Ok(())
            }
            Err(e) => {
                println!("Error creating backup marker: {:?}", e);
                Err(BotError::IoError(e))
            }
        }
    }
}