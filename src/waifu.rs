use crate::config::{BotError, Store, UserProfile};
use chrono::{DateTime, Duration, Timelike, Utc};
use rand::seq::SliceRandom;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use serenity::utils::Color;
use std::collections::HashMap;
use std::fmt;

// Constants for the gacha system
pub const PULL_COST: u32 = 250;
pub const MESSAGE_DIAMONDS: u32 = 10;
pub const PASSIVE_INCOME_RATE: u32 = 20; // diamonds per 10 minutes
pub const PASSIVE_INCOME_INTERVAL: i64 = 10; // minutes
pub const MAX_PASSIVE_INCOME_HOURS: i64 = 12; // Cap passive income at 12 hours to prevent too many diamonds

// Tier requirements and colors
pub const TIER_A_COLOR: u32 = 0x1ABC9C; // Turquoise
pub const TIER_S_COLOR: u32 = 0x3498DB; // Blue
pub const TIER_SS_COLOR: u32 = 0x9B59B6; // Purple
pub const TIER_SSR_COLOR: u32 = 0xE91E63; // Pink/Red

// Enum to represent waifu rarity tiers
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum WaifuTier {
    A,
    S,
    SS,
    SSR,
}

impl WaifuTier {
    // Convert count to tier
    pub fn from_count(count: u32) -> Self {
        match count {
            0..=1 => WaifuTier::A,
            2..=4 => WaifuTier::S,
            5..=9 => WaifuTier::SS,
            _ => WaifuTier::SSR,
        }
    }

    // Get color for tier
    pub fn color(&self) -> u32 {
        match self {
            WaifuTier::A => TIER_A_COLOR,
            WaifuTier::S => TIER_S_COLOR,
            WaifuTier::SS => TIER_SS_COLOR,
            WaifuTier::SSR => TIER_SSR_COLOR,
        }
    }

    // Get formatted name with color/emoji
    pub fn display_name(&self) -> String {
        match self {
            WaifuTier::A => "🟢 A".to_string(),
            WaifuTier::S => "🔵 S".to_string(),
            WaifuTier::SS => "🟣 SS".to_string(),
            WaifuTier::SSR => "🌟 SSR".to_string(),
        }
    }

    // Default rarity distribution for pulls
    pub fn random_tier() -> Self {
        let mut rng = thread_rng();
        let roll: f64 = rng.gen();

        match roll {
            x if x < 0.60 => WaifuTier::A,  // 60%
            x if x < 0.85 => WaifuTier::S,  // 25%
            x if x < 0.97 => WaifuTier::SS, // 12%
            _ => WaifuTier::SSR,            // 3%
        }
    }
}

impl fmt::Display for WaifuTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WaifuTier::A => write!(f, "A"),
            WaifuTier::S => write!(f, "S"),
            WaifuTier::SS => write!(f, "SS"),
            WaifuTier::SSR => write!(f, "SSR"),
        }
    }
}

// Main Waifu data structure
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Waifu {
    pub id: u64,
    pub name: String,
    pub image_url: String,
    pub description: String,
    pub base_tier: WaifuTier,
    pub added_at: DateTime<Utc>,
    pub added_by: u64,
}

// User's waifu collection entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserWaifu {
    pub waifu_id: u64,
    pub count: u32,
    pub first_obtained: DateTime<Utc>,
    pub last_obtained: DateTime<Utc>,
    pub favorite: bool,
}

// Pull history for analytics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullHistory {
    pub user_id: u64,
    pub waifu_id: u64,
    pub timestamp: DateTime<Utc>,
    pub tier: WaifuTier,
}

// Gacha system configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GachaConfig {
    pub enabled: bool,
    pub gacha_channel: Option<u64>,
    pub daily_free_pulls: u32,
    pub pity_system: bool,
    pub pity_counter: u32,         // How many pulls until guaranteed SSR
    pub banner_waifu: Option<u64>, // Featured waifu with higher rates
}

impl Default for GachaConfig {
    fn default() -> Self {
        GachaConfig {
            enabled: true,
            gacha_channel: None,
            daily_free_pulls: 10,
            pity_system: true,
            pity_counter: 50,
            banner_waifu: None,
        }
    }
}

// Struct to track user gacha stats
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserGachaStats {
    pub user_id: u64,
    pub total_pulls: u32,
    pub current_pity: u32,
    pub last_diamond_check: DateTime<Utc>,
    pub last_daily_claim: Option<DateTime<Utc>>,
    pub free_pulls_remaining: u32,
}

// Pull result to return to the user
#[derive(Clone, Debug)]
pub struct PullResult {
    pub waifu: Waifu,
    pub is_new: bool,
    pub current_count: u32,
    pub tier: WaifuTier,
    pub tier_up: bool,
    pub diamonds_spent: u32,
    pub diamonds_remaining: u32,
}

// Extension to the Store for gacha functionality
impl Store {
    // Initialize gacha system
    pub async fn init_gacha_system(&self) -> Result<(), BotError> {
        println!("Initializing gacha system");

        // Check if gacha config exists, create default if not
        if let Ok(None) = self.db_get("gacha_config").await {
            let config = GachaConfig::default();
            let serialized = bincode::serialize(&config)
                .map_err(|e| BotError::SerializationError(e.to_string()))?;

            self.db_insert("gacha_config", &serialized)
                .await
                .map_err(|e| BotError::DatabaseError(e.to_string()))?;

            println!("Created default gacha configuration");
        }

        Ok(())
    }

    // Get gacha configuration
    pub async fn get_gacha_config(&self) -> Result<GachaConfig, BotError> {
        match self.db_get("gacha_config").await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string())),
            Ok(None) => {
                // Create default config if none exists
                let config = GachaConfig::default();
                let serialized = bincode::serialize(&config)
                    .map_err(|e| BotError::SerializationError(e.to_string()))?;

                self.db_insert("gacha_config", &serialized)
                    .await
                    .map_err(|e| BotError::DatabaseError(e.to_string()))?;

                Ok(config)
            }
            Err(e) => Err(BotError::DatabaseError(e.to_string())),
        }
    }

    // Save gacha configuration
    pub async fn save_gacha_config(&self, config: &GachaConfig) -> Result<(), BotError> {
        let serialized =
            bincode::serialize(config).map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert("gacha_config", &serialized)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        println!("Saved gacha configuration");
        Ok(())
    }

    // Set gacha channel
    pub async fn set_gacha_channel(&self, channel_id: u64) -> Result<(), BotError> {
        let mut config = self.get_gacha_config().await?;
        config.gacha_channel = Some(channel_id);
        self.save_gacha_config(&config).await
    }

    // Add a new waifu to the database
    pub async fn add_waifu(
        &self,
        name: String,
        image_url: String,
        description: String,
        base_tier: WaifuTier,
        added_by: u64,
    ) -> Result<u64, BotError> {
        println!("Adding new waifu: {}", name);

        // Generate a unique ID
        let waifu_id = Utc::now().timestamp_millis() as u64;

        let waifu = Waifu {
            id: waifu_id,
            name,
            image_url,
            description,
            base_tier,
            added_at: Utc::now(),
            added_by,
        };

        let serialized =
            bincode::serialize(&waifu).map_err(|e| BotError::SerializationError(e.to_string()))?;

        let key = format!("waifu:{}", waifu_id);
        self.db_insert(&key, &serialized)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        // Also add to waifu index
        let index_key = "waifu_index";
        let mut waifu_ids: Vec<u64> = match self.db_get(index_key).await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string()))?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(BotError::DatabaseError(e.to_string())),
        };

        waifu_ids.push(waifu_id);

        let serialized_index = bincode::serialize(&waifu_ids)
            .map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert(index_key, &serialized_index)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        println!("Successfully added waifu with ID: {}", waifu_id);
        Ok(waifu_id)
    }

    // Get waifu by ID
    pub async fn get_waifu(&self, waifu_id: u64) -> Result<Option<Waifu>, BotError> {
        let key = format!("waifu:{}", waifu_id);
        match self.db_get(&key).await {
            Ok(Some(data)) => {
                let waifu = bincode::deserialize(&data)
                    .map_err(|e| BotError::DeserializationError(e.to_string()))?;
                Ok(Some(waifu))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(BotError::DatabaseError(e.to_string())),
        }
    }

    // Get waifu by name (partial match)
    pub async fn find_waifu_by_name(&self, name: &str) -> Result<Vec<Waifu>, BotError> {
        let waifu_ids = self.get_all_waifu_ids().await?;
        let mut matching_waifus = Vec::new();

        let name_lower = name.to_lowercase();

        for id in waifu_ids {
            if let Some(waifu) = self.get_waifu(id).await? {
                if waifu.name.to_lowercase().contains(&name_lower) {
                    matching_waifus.push(waifu);
                }
            }
        }

        Ok(matching_waifus)
    }

    // Get all waifu IDs
    pub async fn get_all_waifu_ids(&self) -> Result<Vec<u64>, BotError> {
        match self.db_get("waifu_index").await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string())),
            Ok(None) => Ok(Vec::new()),
            Err(e) => Err(BotError::DatabaseError(e.to_string())),
        }
    }

    // Get all waifus
    pub async fn get_all_waifus(&self) -> Result<Vec<Waifu>, BotError> {
        let waifu_ids = self.get_all_waifu_ids().await?;
        let mut waifus = Vec::new();

        for id in waifu_ids {
            if let Some(waifu) = self.get_waifu(id).await? {
                waifus.push(waifu);
            }
        }

        Ok(waifus)
    }

    // Delete a waifu
    pub async fn delete_waifu(&self, waifu_id: u64) -> Result<bool, BotError> {
        // Remove from index first
        let index_key = "waifu_index";
        let mut waifu_ids: Vec<u64> = match self.db_get(index_key).await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string()))?,
            Ok(None) => return Ok(false), // No index, nothing to delete
            Err(e) => return Err(BotError::DatabaseError(e.to_string())),
        };

        let original_len = waifu_ids.len();
        waifu_ids.retain(|&id| id != waifu_id);

        if waifu_ids.len() == original_len {
            return Ok(false); // Waifu not in index
        }

        // Update index
        let serialized_index = bincode::serialize(&waifu_ids)
            .map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert(index_key, &serialized_index)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        // Delete waifu entry
        let key = format!("waifu:{}", waifu_id);
        self.db_remove(&key)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        // Flush changes
        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        println!("Successfully deleted waifu with ID: {}", waifu_id);
        Ok(true)
    }

    //Discard a waifu
    pub async fn discard_waifu(
        &self,
        user_id: u64,
        waifu_name: &str,
    ) -> Result<Option<(Waifu, u32)>, BotError> {
        // Get user's collection
        let mut collection = self.get_user_collection(user_id).await?;

        if collection.is_empty() {
            return Ok(None); // User has no waifus
        }

        // Find all waifus that match the name
        let mut matches = Vec::new();
        for (waifu_id, user_waifu) in &collection {
            if let Ok(Some(waifu)) = self.get_waifu(*waifu_id).await {
                if waifu
                    .name
                    .to_lowercase()
                    .contains(&waifu_name.to_lowercase())
                {
                    matches.push((waifu, user_waifu.clone()));
                }
            }
        }

        // If no matches found
        if matches.is_empty() {
            return Ok(None);
        }

        // If multiple matches, use the one with the highest count
        matches.sort_by(|a, b| b.1.count.cmp(&a.1.count));

        // Get the first match (highest count)
        let (waifu, user_waifu) = matches[0].clone();

        // Reduce count by 1
        if user_waifu.count > 1 {
            // Update the collection - store the new count before saving
            let new_count = {
                if let Some(entry) = collection.get_mut(&waifu.id) {
                    entry.count -= 1;
                    entry.count // Get the new count value
                } else {
                    return Ok(None); // This shouldn't happen
                }
            };

            // Save the updated collection
            self.save_user_collection(user_id, &collection).await?;

            return Ok(Some((waifu, new_count)));
        } else {
            // Remove the waifu if count would go to 0
            collection.remove(&waifu.id);

            // Save the updated collection
            self.save_user_collection(user_id, &collection).await?;

            return Ok(Some((waifu, 0)));
        }
    }

    // Update a waifu's info
    pub async fn update_waifu(&self, waifu: &Waifu) -> Result<(), BotError> {
        let key = format!("waifu:{}", waifu.id);

        let serialized =
            bincode::serialize(waifu).map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert(&key, &serialized)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        println!("Updated waifu: {}", waifu.name);
        Ok(())
    }

    // Get user's gacha stats
    pub async fn get_user_gacha_stats(&self, user_id: u64) -> Result<UserGachaStats, BotError> {
        let key = format!("user_gacha:{}", user_id);
        match self.db_get(&key).await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string())),
            Ok(None) => {
                // Create new stats
                let stats = UserGachaStats {
                    user_id,
                    total_pulls: 0,
                    current_pity: 0,
                    last_diamond_check: Utc::now(),
                    last_daily_claim: None,
                    free_pulls_remaining: 0,
                };
                Ok(stats)
            }
            Err(e) => Err(BotError::DatabaseError(e.to_string())),
        }
    }

    // Save user's gacha stats
    pub async fn save_user_gacha_stats(&self, stats: &UserGachaStats) -> Result<(), BotError> {
        let key = format!("user_gacha:{}", stats.user_id);

        let serialized =
            bincode::serialize(stats).map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert(&key, &serialized)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    // Get user's waifu collection
    pub async fn get_user_collection(
        &self,
        user_id: u64,
    ) -> Result<HashMap<u64, UserWaifu>, BotError> {
        let key = format!("user_collection:{}", user_id);
        match self.db_get(&key).await {
            Ok(Some(data)) => bincode::deserialize(&data)
                .map_err(|e| BotError::DeserializationError(e.to_string())),
            Ok(None) => Ok(HashMap::new()),
            Err(e) => Err(BotError::DatabaseError(e.to_string())),
        }
    }

    // Save user's waifu collection
    pub async fn save_user_collection(
        &self,
        user_id: u64,
        collection: &HashMap<u64, UserWaifu>,
    ) -> Result<(), BotError> {
        let key = format!("user_collection:{}", user_id);

        let serialized = bincode::serialize(collection)
            .map_err(|e| BotError::SerializationError(e.to_string()))?;

        self.db_insert(&key, &serialized)
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        self.db_flush()
            .await
            .map_err(|e| BotError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    // Get user's diamonds with passive income calculation
    pub async fn get_user_diamonds(&self, user_id: u64) -> Result<u32, BotError> {
        // Check cache first for the profile
        let profile = match self.get_user_profile(user_id).await {
            Some(profile) => profile,
            None => {
                // Create new profile with default values
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
        };

        // Get gacha stats for passive income calculation
        let mut gacha_stats = self.get_user_gacha_stats(user_id).await?;

        // Calculate passive income since last check
        let now = Utc::now();
        let time_passed = now.signed_duration_since(gacha_stats.last_diamond_check);

        // Cap at MAX_PASSIVE_INCOME_HOURS to prevent excessive diamonds from long absences
        let minutes_passed =
            std::cmp::min(time_passed.num_minutes(), MAX_PASSIVE_INCOME_HOURS * 60);
        let intervals = minutes_passed / PASSIVE_INCOME_INTERVAL;

        if intervals > 0 {
            // Calculate earned diamonds
            let earned_diamonds = (intervals as u32) * PASSIVE_INCOME_RATE;

            // Create a mutable copy for updating
            let mut profile_copy = profile.clone();
            profile_copy.diamonds += earned_diamonds;

            // Update last diamond check time
            gacha_stats.last_diamond_check = now;
            self.save_user_gacha_stats(&gacha_stats).await?;

            // Save updated profile with batch processing
            self.save_user_profile(&profile_copy).await;

            Ok(profile_copy.diamonds)
        } else {
            Ok(profile.diamonds)
        }
    }

    // OPTIMIZED: Add diamonds to user with caching
    pub async fn add_diamonds(&self, user_id: u64, amount: u32) -> Result<u32, BotError> {
        // Get user profile or create one if it doesn't exist
        let mut profile = match self.get_user_profile(user_id).await {
            Some(profile) => profile,
            None => {
                // Create a new profile with default values
                println!("Creating new profile for user {} in add_diamonds", user_id);
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
        };

        // Calculate passive income first
        let current_diamonds = match self.get_user_diamonds(user_id).await {
            Ok(diamonds) => diamonds,
            Err(_) => profile.diamonds, // Fallback to stored value if get_user_diamonds fails
        };

        // Set the current diamonds (including passive income)
        profile.diamonds = current_diamonds;

        // Add the new diamonds
        profile.diamonds += amount;

        // Save profile with batch processing
        self.save_user_profile(&profile).await;

        Ok(profile.diamonds)
    }

    // Take diamonds from user (for pulls)
    pub async fn spend_diamonds(&self, user_id: u64, amount: u32) -> Result<bool, BotError> {
        // Get current diamonds
        let current = self.get_user_diamonds(user_id).await?;

        if current < amount {
            return Ok(false); // Not enough diamonds
        }

        // Get user profile
        if let Some(mut profile) = self.get_user_profile(user_id).await {
            profile.diamonds = current - amount;
            self.save_user_profile(&profile).await;
            Ok(true)
        } else {
            Ok(false) // No profile found
        }
    }

    // Track message for diamonds
    pub async fn track_message_diamonds(&self, user_id: u64) -> Result<(), BotError> {
        // Add message diamonds - directly update in memory then queue for batch save
        let diamonds = match self.get_user_profile(user_id).await {
            Some(mut profile) => {
                profile.diamonds += MESSAGE_DIAMONDS;
                self.save_user_profile(&profile).await;
                Ok(())
            },
            None => {
                // Create new profile with initial diamonds and save
                let mut profile = Self::create_new_profile(user_id);
                profile.diamonds = MESSAGE_DIAMONDS;
                self.save_user_profile(&profile).await;
                Ok(())
            }
        };
        
        diamonds
    }

    // Perform a gacha pull
    pub async fn pull_waifu(&self, user_id: u64, free_pull: bool) -> Result<PullResult, BotError> {
        // Get available waifus
        let waifus = self.get_all_waifus().await?;
        if waifus.is_empty() {
            return Err(BotError::DatabaseError(
                "No waifus available for pulling".to_string(),
            ));
        }

        // Get user gacha stats
        let mut gacha_stats = self.get_user_gacha_stats(user_id).await?;

        // Get gacha config for pity system
        let gacha_config = self.get_gacha_config().await?;

        // Determine pull tier based on pity or random
        let pull_tier =
            if gacha_config.pity_system && gacha_stats.current_pity >= gacha_config.pity_counter {
                println!("User {} hit pity counter, guaranteeing SSR pull", user_id);
                WaifuTier::SSR
            } else {
                WaifuTier::random_tier()
            };

        // Filter waifus by tier
        let tier_waifus: Vec<Waifu> = waifus
            .iter()
            .filter(|w| w.base_tier == pull_tier)
            .cloned() // Clone each waifu to own them
            .collect();

        // If no waifus of the selected tier, use any waifu
        let selected_waifu = if tier_waifus.is_empty() {
            waifus
                .choose(&mut thread_rng())
                .ok_or(BotError::DatabaseError(
                    "Failed to choose a random waifu".to_string(),
                ))?
                .clone()
        } else {
            tier_waifus
                .choose(&mut thread_rng())
                .ok_or(BotError::DatabaseError(
                    "Failed to choose a random waifu".to_string(),
                ))?
                .clone()
        };

        // Check if user has enough diamonds (if not a free pull)
        let diamonds_spent = if free_pull { 0 } else { PULL_COST };
        let diamonds_remaining;

        if !free_pull {
            // Spend diamonds
            let success = self.spend_diamonds(user_id, PULL_COST).await?;
            if !success {
                return Err(BotError::DatabaseError(format!(
                    "Not enough diamonds. Required: {}",
                    PULL_COST
                )));
            }

            // Get remaining diamonds
            diamonds_remaining = self.get_user_diamonds(user_id).await?;
        } else {
            // Just get current diamonds without spending
            diamonds_remaining = self.get_user_diamonds(user_id).await?;

            // Decrement free pulls
            gacha_stats.free_pulls_remaining = gacha_stats.free_pulls_remaining.saturating_sub(1);
        }

        // Update user's waifu collection
        let mut collection = self.get_user_collection(user_id).await?;
        let is_new = !collection.contains_key(&selected_waifu.id);
        let now = Utc::now();

        let current_count = if let Some(user_waifu) = collection.get_mut(&selected_waifu.id) {
            // User already has this waifu, increment count
            user_waifu.count += 1;
            user_waifu.last_obtained = now;
            user_waifu.count
        } else {
            // First time getting this waifu
            collection.insert(
                selected_waifu.id,
                UserWaifu {
                    waifu_id: selected_waifu.id,
                    count: 1,
                    first_obtained: now,
                    last_obtained: now,
                    favorite: false,
                },
            );
            1
        };

        // Save updated collection
        self.save_user_collection(user_id, &collection).await?;

        // Update gacha stats
        gacha_stats.total_pulls += 1;

        // Update pity counter
        if pull_tier == WaifuTier::SSR {
            // Reset pity after getting SSR
            gacha_stats.current_pity = 0;
        } else {
            // Increment pity counter
            gacha_stats.current_pity += 1;
        }

        // Save updated gacha stats
        self.save_user_gacha_stats(&gacha_stats).await?;

        // Determine if this pull resulted in a tier upgrade
        let old_tier = WaifuTier::from_count(current_count - 1);
        let new_tier = WaifuTier::from_count(current_count);
        let tier_up = old_tier != new_tier;

        // Return pull result
        Ok(PullResult {
            waifu: selected_waifu,
            is_new,
            current_count,
            tier: new_tier,
            tier_up,
            diamonds_spent,
            diamonds_remaining,
        })
    }

    // Get user's favorite waifu
    pub async fn get_user_favorite(
        &self,
        user_id: u64,
    ) -> Result<Option<(Waifu, UserWaifu)>, BotError> {
        // Get user collection
        let collection = self.get_user_collection(user_id).await?;

        // Look for explicitly favorited waifu
        for (waifu_id, user_waifu) in &collection {
            if user_waifu.favorite {
                if let Some(waifu) = self.get_waifu(*waifu_id).await? {
                    return Ok(Some((waifu, user_waifu.clone())));
                }
            }
        }

        // If no favorite, find highest tier/count
        let mut best_waifu: Option<(u64, &UserWaifu)> = None;

        for (waifu_id, user_waifu) in &collection {
            if let Some((_, current_best)) = best_waifu {
                if user_waifu.count > current_best.count {
                    best_waifu = Some((*waifu_id, user_waifu));
                }
            } else {
                best_waifu = Some((*waifu_id, user_waifu));
            }
        }

        // Return the best waifu if found
        if let Some((waifu_id, user_waifu)) = best_waifu {
            if let Some(waifu) = self.get_waifu(waifu_id).await? {
                return Ok(Some((waifu, user_waifu.clone())));
            }
        }

        Ok(None)
    }

    // Set a waifu as favorite
    pub async fn set_favorite_waifu(&self, user_id: u64, waifu_id: u64) -> Result<bool, BotError> {
        // Get user collection
        let mut collection = self.get_user_collection(user_id).await?;

        // Check if user has this waifu
        if !collection.contains_key(&waifu_id) {
            return Ok(false);
        }

        // Clear all favorites
        for (_, user_waifu) in &mut collection {
            user_waifu.favorite = false;
        }

        // Set the new favorite
        if let Some(user_waifu) = collection.get_mut(&waifu_id) {
            user_waifu.favorite = true;
        }

        // Save updated collection
        self.save_user_collection(user_id, &collection).await?;

        Ok(true)
    }

    // Check for daily reset
    pub async fn check_daily_reset(&self, user_id: u64) -> Result<u32, BotError> {
        let mut gacha_stats = self.get_user_gacha_stats(user_id).await?;
        let gacha_config = self.get_gacha_config().await?;
        let now = Utc::now();

        let should_reset = match gacha_stats.last_daily_claim {
            Some(last_claim) => {
                // Check if it's a different day (UTC)
                last_claim.date() != now.date()
            }
            None => true, // Never claimed before
        };

        if should_reset {
            gacha_stats.free_pulls_remaining = gacha_config.daily_free_pulls;
            gacha_stats.last_daily_claim = Some(now);
            self.save_user_gacha_stats(&gacha_stats).await?;

            // Add daily login bonus diamonds
            self.add_diamonds(user_id, 100).await?;

            println!("Reset daily pulls for user {}", user_id);
        }

        Ok(gacha_stats.free_pulls_remaining)
    }
}

// Helper functions for displaying gacha info in Discord embeds
pub fn add_waifu_to_embed(embed: &mut serenity::builder::CreateEmbed, waifu: &Waifu, count: u32) {
    let tier = WaifuTier::from_count(count);
    let tier_display = tier.display_name();

    embed
        .title(&waifu.name)
        .description(&waifu.description)
        .color(tier.color())
        .thumbnail(&waifu.image_url)
        .field("Tier", tier_display, true)
        .field("Copies", count.to_string(), true);

    // Add shimmer effect for SSR (via footer text)
    if tier == WaifuTier::SSR {
        embed.footer(|f| f.text("✨ SSR Rarity ✨"));
    }
}

// Create rainbow/cycling color effect for SSR waifus in embeds
pub fn get_ssr_color() -> Color {
    let now = Utc::now();
    let seconds = now.second() as f32;

    // Create a cycling hue (0-360 degrees)
    let hue = (seconds * 6.0) % 360.0;

    // Convert HSV to RGB (simplified, fixed saturation and value)
    let saturation = 1.0;
    let value = 1.0;

    let c = value * saturation;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = value - c;

    let (r, g, b) = match hue as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let (r, g, b) = (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    );

    Color::from_rgb(r, g, b)
}
