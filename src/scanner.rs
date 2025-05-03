use reqwest;
use serde::{Deserialize, Serialize};
use serenity::builder::{CreateEmbed, CreateEmbedFooter};
use serenity::model::channel::Message;
use serenity::model::id::ChannelId;
use serenity::utils::Color;
use serenity::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time;
use async_trait::async_trait;
use std::error::Error as StdError;
use std::fmt;
use rand::Rng;
use chrono::Utc;

// Define helper functions for address validation
fn is_solana_address(text: &str) -> bool {
    text.len() >= 32 && text.len() <= 44 && 
    text.chars().all(|c| c.is_alphanumeric())
}

fn is_ethereum_address(text: &str) -> bool {
    text.len() == 42 && 
    text.starts_with("0x") && 
    text[2..].chars().all(|c| c.is_ascii_hexdigit())
}

// Simple match structures for pattern matching
struct SimpleMatch<'a> {
    matched_text: &'a str,
}

impl<'a> SimpleMatch<'a> {
    fn get(&self, _idx: usize) -> Option<SimpleMatchItem<'a>> {
        Some(SimpleMatchItem { matched_text: self.matched_text })
    }
}

struct SimpleMatchItem<'a> {
    matched_text: &'a str,
}

// API endpoints
const SOLSCAN_API_URL: &str = "https://public-api.solscan.io";
const EXPLORER_URL: &str = "https://explorer.solana.com";
const COINGECKO_API_URL: &str = "https://api.coingecko.com/api/v3";
const POLYGON_API_URL: &str = "https://api.polygonscan.com/api";
const JUPITER_TOKEN_API: &str = "https://token-list-api.jup.ag/v1";
const JUPITER_PRICE_API: &str = "https://api.jup.ag/price/v2";
const JUPITER_API_URL: &str = "https://api.jup.ag";
const JUPITER_POPULAR_TOKENS_URL: &str = "https://token.jup.ag/strict";
const JUPITER_TOKEN_API_URL: &str = "https://api.jup.ag/tokens/v1";

// Cache expiry times - separate for different data types
const TOKEN_CACHE_SECONDS: u64 = 600; // 10 minutes
const ADDRESS_CACHE_SECONDS: u64 = 600; // 10 minutes
const TRENDING_CACHE_SECONDS: u64 = 600; // 10 minutes
const WALLET_CACHE_SECONDS: u64 = 900; // 15 minutes
const NFT_CACHE_SECONDS: u64 = 3600; // 1 hour

// Custom error type that implements Send + Sync
#[derive(Debug)]
pub struct ScannerError {
    message: String,
}

impl ScannerError {
    fn new(message: &str) -> Self {
        ScannerError { message: message.to_string() }
    }
}

impl fmt::Display for ScannerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl StdError for ScannerError {}

// Emoji definitions for consistent UI
struct Emojis;
impl Emojis {
    // Currency/Value emojis
    const SOL: &'static str = "◎"; // Sol symbol
    const PRICE: &'static str = "💰";
    const CHANGE: &'static str = "📊";
    const VOLUME: &'static str = "🔄";
    const MARKET_CAP: &'static str = "🧮";
    const SUPPLY: &'static str = "🗃️";
    
    // Trend indicators
    const UP: &'static str = "📈";
    const DOWN: &'static str = "📉";
    const NEUTRAL: &'static str = "➖";
    const MOON: &'static str = "🚀";
    const DUMP: &'static str = "💩";
    
    // Token/NFT categories
    const TOKEN: &'static str = "🪙";
    const NFT: &'static str = "🖼️";
    const WALLET: &'static str = "👛";
    const CONTRACT: &'static str = "📜";
    
    // Chains
    const SOLANA: &'static str = "☀️";
    const ETH: &'static str = "⟠";
    const POLYGON: &'static str = "🔷";
    
    // Misc
    const INFO: &'static str = "ℹ️";
    const WARNING: &'static str = "⚠️";
    const ERROR: &'static str = "❌";
    const SUCCESS: &'static str = "✅";
    const WAIT: &'static str = "⏳";
    const BONK: &'static str = "🦴";
    const HOT: &'static str = "🔥";
    const CHART: &'static str = "📊";
    const LIQUIDITY: &'static str = "💧";
    const BOBA: &'static str = "🧋";  // Hikari's special emoji
}

// Add Hikari's personality traits
struct HikariStyle;
impl HikariStyle {
    // Random exclamations/reactions for Hikari to use
    const EXCITED: &'static [&'static str] = &[
        "WAGMI fr fr! 🚀",
        "gonna moon so hard bestie! 💎🙌",
        "no cap, this is fire! 🔥",
        "straight bussin! 🔥",
        "diamond hands vibes only! 💎",
        "bullish AF on this! 📈",
    ];
    
    const DISAPPOINTED: &'static [&'static str] = &[
        "big yikes on this one... 💀",
        "down bad, but HODL! 💪",
        "not the vibe rn... 😬",
        "paper hands be sellin' fr 📉",
        "bearish szn, but we bounce back! 🐻",
        "no FUD but this ain't it chief 🤧",
    ];
    
    // Returns a random excited phrase
    fn random_excited() -> &'static str {
        let idx = rand::thread_rng().gen_range(0..HikariStyle::EXCITED.len());
        HikariStyle::EXCITED[idx]
    }
    
    // Returns a random disappointed phrase
    fn random_disappointed() -> &'static str {
        let idx = rand::thread_rng().gen_range(0..HikariStyle::DISAPPOINTED.len());
        HikariStyle::DISAPPOINTED[idx]
    }
    
    // Adds Hikari's touch to embed footers
    fn apply_footer(footer: &mut CreateEmbedFooter) -> &mut CreateEmbedFooter {
        footer.text(format!("powered by Hikari • {} • solana szn", HikariStyle::random_excited()))
    }
    
    // Returns appropriate color based on change percentage
    fn get_trend_color(change_pct: f64) -> Color {
        match change_pct {
            c if c > 20.0 => Color::from_rgb(114, 9, 183),   // Purple (mooning)
            c if c > 5.0 => Color::from_rgb(0, 255, 0),      // Green (strong up)
            c if c > 0.0 => Color::from_rgb(0, 200, 0),      // Light green (up)
            c if c > -5.0 => Color::from_rgb(255, 165, 0),   // Orange (small down)
            c if c > -20.0 => Color::from_rgb(255, 0, 0),    // Red (down)
            _ => Color::from_rgb(139, 0, 0),                 // Dark red (crashing)
        }
    }
    
    // Returns trend emoji based on change percentage
    fn get_trend_emoji(change_pct: f64) -> &'static str {
        match change_pct {
            c if c > 20.0 => Emojis::MOON,
            c if c > 0.0 => Emojis::UP,
            c if c > -10.0 => Emojis::DOWN,
            _ => Emojis::DUMP,
        }
    }
}

// Data structures
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenData {
    pub symbol: String,
    pub name: Option<String>,
    pub price: f64,
    pub price_change_24h: Option<f64>,
    pub volume_24h: Option<f64>,
    pub market_cap: Option<f64>,
    pub fully_diluted_valuation: Option<f64>,
    pub total_supply: Option<f64>,
    pub max_supply: Option<f64>,
    pub last_updated: String,
    pub address: Option<String>,
    pub chain: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrendingToken {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub price_change_24h: f64,
    pub volume_24h: f64,
    pub created_at: Option<String>,
    pub chain: String,
    pub market_cap: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletInfo {
    pub address: String,
    pub balance_sol: f64,
    pub balance_usd: f64,
    pub tokens: Vec<WalletToken>,
    pub nfts: Vec<String>,
    pub total_value_usd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletToken {
    pub symbol: String,
    pub amount: f64,
    pub value_usd: f64,
    pub token_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NFTInfo {
    pub name: String,
    pub collection: Option<String>,
    pub image_url: Option<String>,
    pub token_address: String,
    pub owner: Option<String>,
    pub floor_price: Option<f64>,
    pub last_sale_price: Option<f64>,
}

// Cache structs with generic typing
struct CachedData<T> {
    timestamp: Instant,
    data: T,
}

// Main scanner struct
pub struct HikariScanner {
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<HashMap<String, CachedData<TokenData>>>>,
    address_cache: Arc<RwLock<HashMap<String, CachedData<serde_json::Value>>>>,
    trending_cache: Arc<RwLock<Vec<TrendingToken>>>,
    last_trending_update: Arc<RwLock<Instant>>,
    wallet_cache: Arc<RwLock<HashMap<String, CachedData<WalletInfo>>>>,
    nft_cache: Arc<RwLock<HashMap<String, CachedData<NFTInfo>>>>,
    api_keys: HashMap<String, String>,
}

impl HikariScanner {
    pub fn new(api_keys: HashMap<String, String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))  // Increased timeout for better reliability
            .user_agent("HikariScanner/1.0")   // Set user agent for better API response
            .build()
            .unwrap_or_default();
            
        // Initialize with expired last_trending_update to force immediate update
        let expired_time = Instant::now() - Duration::from_secs(TRENDING_CACHE_SECONDS + 1);
        
        HikariScanner {
            http_client: client,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
            address_cache: Arc::new(RwLock::new(HashMap::new())),
            trending_cache: Arc::new(RwLock::new(Vec::new())),
            last_trending_update: Arc::new(RwLock::new(expired_time)),
            wallet_cache: Arc::new(RwLock::new(HashMap::new())),
            nft_cache: Arc::new(RwLock::new(HashMap::new())),
            api_keys,
        }
    }

    // Initialize background tasks for data updates
    pub async fn start_background_tasks(self: Arc<Self>, ctx: Arc<Context>) {
        println!("Starting Hikari Scanner background tasks");
        
        // Background task to update trending tokens
        {
            let scanner_clone = Arc::clone(&self);
            tokio::spawn(async move {
                println!("Starting trending tokens update task");
                let mut interval = time::interval(Duration::from_secs(TRENDING_CACHE_SECONDS));
                
                // Force immediate first update
                if let Err(e) = scanner_clone.update_trending_tokens(false).await {
                    println!("Initial trending tokens update failed: {}", e);
                } else {
                    println!("Initial trending tokens update completed successfully");
                }
                
                loop {
                    interval.tick().await;
                    if let Err(e) = scanner_clone.update_trending_tokens(false).await {
                        println!("Error updating trending tokens: {}", e);
                    } else {
                        println!("Trending tokens update completed successfully");
                    }
                }
            });
        }
        
        // Background task to clean up expired caches
        {
            let scanner_clone = Arc::clone(&self);
            tokio::spawn(async move {
                println!("Starting cache cleanup task");
                let mut interval = time::interval(Duration::from_secs(300)); // Every 5 minutes
                loop {
                    interval.tick().await;
                    scanner_clone.clean_expired_caches().await;
                }
            });
        }
        
        println!("Hikari scanner background tasks started 🧋");
    }
    
    // Clean expired caches method
    async fn clean_expired_caches(&self) {
        println!("Cleaning expired caches");
        let mut cleaned_count = 0;
        
        // Clean token cache
        {
            let mut token_cache = self.token_cache.write().await;
            let before_count = token_cache.len();
            token_cache.retain(|_, cached| {
                cached.timestamp.elapsed().as_secs() < TOKEN_CACHE_SECONDS
            });
            cleaned_count += before_count - token_cache.len();
        }
        
        // Clean address cache
        {
            let mut address_cache = self.address_cache.write().await;
            let before_count = address_cache.len();
            address_cache.retain(|_, cached| {
                cached.timestamp.elapsed().as_secs() < ADDRESS_CACHE_SECONDS
            });
            cleaned_count += before_count - address_cache.len();
        }
        
        // Clean wallet cache
        {
            let mut wallet_cache = self.wallet_cache.write().await;
            let before_count = wallet_cache.len();
            wallet_cache.retain(|_, cached| {
                cached.timestamp.elapsed().as_secs() < WALLET_CACHE_SECONDS
            });
            cleaned_count += before_count - wallet_cache.len();
        }
        
        // Clean NFT cache
        {
            let mut nft_cache = self.nft_cache.write().await;
            let before_count = nft_cache.len();
            nft_cache.retain(|_, cached| {
                cached.timestamp.elapsed().as_secs() < NFT_CACHE_SECONDS
            });
            cleaned_count += before_count - nft_cache.len();
        }
        
        if cleaned_count > 0 {
            println!("Cleaned {} expired cache entries 🧹", cleaned_count);
        }
    }

    // Main message handler
    pub async fn handle_message(&self, ctx: Context, msg: Message) -> bool {
        // Skip bot messages
        if msg.author.bot {
            return false;
        }

        let content = msg.content.to_lowercase();
        
        // Check for address patterns in the message first
        if let Some(address) = self.extract_address_from_message(&msg.content) {
            println!("Found address in message: {}", address);
            return self.respond_to_address(&ctx, msg.channel_id, &address).await;
        }
        
        // Check for commands
        if content.starts_with("!sol") || content.starts_with("!solana") || 
           content.starts_with("!crypto") || content.starts_with("!coin") || 
           content.starts_with("!price") || content.starts_with("!token") ||
           content.starts_with("!wallet") || content.starts_with("!nft") ||
           content.starts_with("!trending") {
           
            return self.handle_commands(&ctx, &msg).await;
        }
        
        // Check for direct mentions of coins
        if content.contains("bitcoin") || content.contains("btc") ||
           content.contains("ethereum") || content.contains("eth") ||
           content.contains("solana") || content.contains("sol") {
            
            return self.handle_coin_mention(&ctx, &msg).await;
        }

        false
    }
    
    // Extract a blockchain address from a message
    fn extract_address_from_message(&self, message: &str) -> Option<String> {
        // Check for Solana or Ethereum address pattern in each word
        for word in message.split_whitespace() {
            let trimmed = word.trim_matches(|c| c == '`' || c == '\'' || c == '"' || c == '<' || c == '>');
            if is_solana_address(trimmed) {
                return Some(trimmed.to_string());
            }
            if is_ethereum_address(trimmed) {
                return Some(trimmed.to_string());
            }
        }
        
        None
    }
    
    // Respond to address found in a message
    async fn respond_to_address(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        // Determine address type and respond accordingly
        if is_solana_address(address) {
            return self.send_solana_address_info(ctx, channel_id, address).await;
        } else if is_ethereum_address(address) {
            return self.send_eth_address_info(ctx, channel_id, address).await;
        }
        
        false
    }
    
    // Add this new function to get detailed token information
    async fn get_token_details(&self, address: &str) -> Option<serde_json::Value> {
        let url = format!("{}/token/{}", JUPITER_TOKEN_API_URL, address);
        println!("Fetching token details from: {}", url);
        
        match self.http_client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await {
                
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<serde_json::Value>().await {
                        Ok(data) => {
                            println!("Successfully retrieved token details for: {}", address);
                            Some(data)
                        },
                        Err(e) => {
                            println!("Error parsing token details: {}", e);
                            None
                        }
                    }
                } else {
                    println!("Token API returned error: {}", response.status());
                    None
                }
            },
            Err(e) => {
                println!("Error connecting to Token API: {}", e);
                None
            }
        }
    }
    
    // FIXED: Implemented the missing get_token_price function
    async fn get_token_price(&self, token_id: &str) -> Result<TokenData, Box<dyn StdError + Send + Sync>> {
        println!("Getting token price for: {}", token_id);
        
        // Check cache first
        {
            let token_cache = self.token_cache.read().await;
            if let Some(cached) = token_cache.get(token_id) {
                if cached.timestamp.elapsed().as_secs() < TOKEN_CACHE_SECONDS {
                    println!("Cache hit for token: {}", token_id);
                    return Ok(cached.data.clone());
                }
            }
        }
        
        // Try Jupiter API first
        match self.fetch_token_from_jupiter(token_id).await {
            Ok(token) => {
                // Cache the result
                let mut token_cache = self.token_cache.write().await;
                token_cache.insert(token_id.to_string(), CachedData {
                    timestamp: Instant::now(),
                    data: token.clone(),
                });
                
                println!("Successfully retrieved token price from Jupiter: {}", token_id);
                return Ok(token);
            },
            Err(jupiter_err) => {
                println!("Jupiter API failed, trying CoinGecko. Error: {}", jupiter_err);
                
                // Fall back to CoinGecko API
                match self.fetch_token_from_coingecko(token_id).await {
                    Ok(token) => {
                        // Cache the result
                        let mut token_cache = self.token_cache.write().await;
                        token_cache.insert(token_id.to_string(), CachedData {
                            timestamp: Instant::now(),
                            data: token.clone(),
                        });
                        
                        println!("Successfully retrieved token price from CoinGecko: {}", token_id);
                        return Ok(token);
                    },
                    Err(coingecko_err) => {
                        println!("Both API calls failed. Returning error from CoinGecko: {}", coingecko_err);
                        return Err(coingecko_err);
                    }
                }
            }
        }
    }

    // Update send_token_price to use the Token API data when available
    async fn send_token_price(&self, ctx: &Context, channel_id: ChannelId, token_id: &str) -> bool {
        println!("Sending token price info for: {}", token_id);
        
        let token_id = token_id.trim();
        
        // First try to get detailed token info from Jupiter Token API
        let token_details = self.get_token_details(token_id).await;
        
        // Then get price data (might need a fallback)
        let price_result = self.get_token_price(token_id).await;
        
        match (token_details, price_result) {
            (Some(details), Ok(price_data)) => {
                // We have both detailed info and price data - create a comprehensive display
                self.send_detailed_token_info(ctx, channel_id, &details, &price_data).await
            },
            (Some(details), Err(_)) => {
                // We have token details but no price - try to extract price from details
                let price = details["price"].as_f64().unwrap_or(0.0);
                let price_data = TokenData {
                    symbol: details["symbol"].as_str().unwrap_or("UNKNOWN").to_string(),
                    name: details["name"].as_str().map(|n| n.to_string()),
                    price,
                    price_change_24h: None,
                    volume_24h: details["daily_volume"].as_f64(),
                    market_cap: None,
                    fully_diluted_valuation: None,
                    total_supply: None,
                    max_supply: None,
                    last_updated: Utc::now().to_string(),
                    address: Some(token_id.to_string()),
                    chain: "solana".to_string(),
                };
                
                self.send_detailed_token_info(ctx, channel_id, &details, &price_data).await
            },
            (None, Ok(price_data)) => {
                // Fall back to old method when no details available
                self.send_basic_token_info(ctx, channel_id, &price_data).await
            },
            (None, Err(e)) => {
                println!("Error getting token info: {}", e);
                if let Err(err) = channel_id.say(&ctx.http, 
                    format!("{} Can't find that token, bestie! Maybe check the spelling or try the contract address?", 
                            Emojis::ERROR)).await {
                    println!("Error sending error message: {}", err);
                }
                false
            }
        }
    }

    async fn send_basic_token_info(&self, ctx: &Context, channel_id: ChannelId, token: &TokenData) -> bool {
        let chain_emoji = match token.chain.as_str() {
            "solana" => Emojis::SOLANA,
            "ethereum" => Emojis::ETH,
            "polygon" => Emojis::POLYGON,
            _ => Emojis::TOKEN
        };
        
        let change_pct = token.price_change_24h.unwrap_or(0.0);
        let change_emoji = HikariStyle::get_trend_emoji(change_pct);
        
        let hikari_comment = if change_pct > 10.0 {
            format!("{}! This bad boy's pumping! Time to ape in or what!?", 
                HikariStyle::random_excited())
        } else if change_pct > 0.0 {
            "Steady gains, looking bullish! WAGMI! 💎".to_string()
        } else if change_pct > -10.0 {
            "Just a lil dip... perfect time to buy the bottom fr! 👀".to_string()
        } else {
            format!("{}! Mega dump, but real ones HODL through the bear market!", 
                HikariStyle::random_disappointed())
        };
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{} {} ({}) Price Check", 
                        chain_emoji, 
                        token.name.as_ref().unwrap_or(&token.symbol.to_uppercase()),
                        token.symbol.to_uppercase()))
                    .field(format!("{}Price", Emojis::PRICE), 
                          format!("${:.6}", token.price), 
                          true)
                    .field(format!("{}24h Change", Emojis::CHANGE), 
                          format!("{} {:.2}%", change_emoji, change_pct), 
                          true);
                          
                if let Some(volume) = token.volume_24h {
                    if volume >= 1_000_000_000.0 {
                        embed = embed.field(format!("{}24h Volume", Emojis::VOLUME), 
                            format!("${:.2}B", volume / 1_000_000_000.0), 
                            true);
                    } else {
                        embed = embed.field(format!("{}24h Volume", Emojis::VOLUME), 
                            format!("${:.2}M", volume / 1_000_000.0), 
                            true);
                    }
                }
                
                if let Some(market_cap) = token.market_cap {
                    if market_cap >= 1_000_000_000.0 {
                        embed = embed.field(format!("{}Market Cap", Emojis::MARKET_CAP), 
                            format!("${:.2}B", market_cap / 1_000_000_000.0), 
                            true);
                    } else {
                        embed = embed.field(format!("{}Market Cap", Emojis::MARKET_CAP), 
                            format!("${:.2}M", market_cap / 1_000_000.0), 
                            true);
                    }
                }
                
                if let Some(address) = &token.address {
                    if !address.is_empty() {
                        let explorer_url = if token.chain == "solana" {
                            format!("https://explorer.solana.com/address/{}", address)
                        } else if token.chain == "ethereum" {
                            format!("https://etherscan.io/token/{}", address)
                        } else {
                            "#".to_string()
                        };
                        
                        embed = embed.field(format!("{}Address", Emojis::CONTRACT), 
                            format!("[{}...{}]({})", 
                                &address[0..6], 
                                &address[address.len().saturating_sub(4)..], 
                                explorer_url), 
                            false);
                    }
                }
                
                embed = embed.field(format!("{}Hikari's Take", Emojis::BOBA),
                              hikari_comment,
                              false);
                              
                embed.color(HikariStyle::get_trend_color(change_pct))
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending token price: {}", e);
            return false;
        }
        
        println!("Token price info sent successfully");
        true
    }

    // New method for detailed token information display
    async fn send_detailed_token_info(&self, ctx: &Context, channel_id: ChannelId, details: &serde_json::Value, price_data: &TokenData) -> bool {
        // Extract data from token details
        let symbol = details["symbol"].as_str().unwrap_or_else(|| price_data.symbol.as_str());
        let name = details["name"].as_str().unwrap_or_else(|| price_data.name.as_deref().unwrap_or("Unknown Token"));
        let decimals = details["decimals"].as_i64().unwrap_or(9);
        
        // Get logo if available
        let logo_url = details["logoURI"].as_str().unwrap_or("");
        
        // Calculate market cap if we have info about supply
        let daily_volume = details["daily_volume"].as_f64().unwrap_or_else(|| price_data.volume_24h.unwrap_or(0.0));
        
        // Get token age
        let created_at = details["created_at"].as_str().unwrap_or("");
        let minted_at = details["minted_at"].as_str().unwrap_or("");
        
        // Get creation date and format as relative time
        let age_text = if !minted_at.is_empty() {
            if let Ok(date) = chrono::DateTime::parse_from_rfc3339(minted_at) {
                let now = Utc::now();
                let duration = now.signed_duration_since(date.with_timezone(&Utc));
                
                if duration.num_days() > 365 {
                    format!("{:.1}y", duration.num_days() as f64 / 365.0)
                } else if duration.num_days() > 30 {
                    format!("{}mo", duration.num_days() / 30)
                } else {
                    format!("{}d", duration.num_days())
                }
            } else {
                "Unknown".to_string()
            }
        } else {
            "Unknown".to_string()
        };
        
        // Get tags for additional info
        let tags = if let Some(tags_arr) = details["tags"].as_array() {
            let tags_str: Vec<String> = tags_arr
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            tags_str.join(", ")
        } else {
            "".to_string()
        };
        
        // Get price change
        let change_pct = price_data.price_change_24h.unwrap_or(0.0);
        let change_emoji = HikariStyle::get_trend_emoji(change_pct);
        
        // Security indicators
        let freeze_authority = details["freeze_authority"].is_null();
        let mint_authority = details["mint_authority"].is_null();
        let permanent_delegate = details["permanent_delegate"].is_null();
        
        let security_rating = if freeze_authority && mint_authority && permanent_delegate {
            "✅ Secure"
        } else if !freeze_authority && !mint_authority {
            "⚠️ Caution"
        } else {
            "⚠️ Review"
        };
        
        // Format price with appropriate precision based on value
        let price_display = if price_data.price < 0.000001 {
            format!("${:.10}", price_data.price)
        } else if price_data.price < 0.001 {
            format!("${:.8}", price_data.price)
        } else if price_data.price < 1.0 {
            format!("${:.6}", price_data.price)
        } else {
            format!("${:.4}", price_data.price)
        };
        
        // Estimate market cap if we can
        let market_cap_display = if let Some(market_cap) = price_data.market_cap {
            if market_cap >= 1_000_000_000.0 {
                format!("${:.1}B", market_cap / 1_000_000_000.0)
            } else {
                format!("${:.1}M", market_cap / 1_000_000.0)
            }
        } else if daily_volume > 0.0 {
            // Very rough estimate based on volume
            let est_cap = daily_volume * 5.0; // Super rough calculation
            if est_cap >= 1_000_000_000.0 {
                format!("~${:.1}B", est_cap / 1_000_000_000.0)
            } else {
                format!("~${:.1}M", est_cap / 1_000_000.0)
            }
        } else {
            "Unknown".to_string()
        };
        
        // Create explorer links
        let explorer_links = format!(
            "[Solana Explorer]({}/address/{})\n[Birdeye](https://birdeye.so/token/{}?chain=solana)\n[Dexscreener](https://dexscreener.com/solana/{})\n[Jupiter](https://station.jup.ag/token/{})",
            EXPLORER_URL, price_data.address.as_deref().unwrap_or(symbol),
            price_data.address.as_deref().unwrap_or(symbol),
            price_data.address.as_deref().unwrap_or(symbol),
            price_data.address.as_deref().unwrap_or(symbol)
        );
        
        // FIXED: Corrected the title formatting issue
let title = if change_pct != 0.0 {
    format!("{} {} ({}) [{} {}{}%]", 
        if change_pct > 0.0 { "🟢" } else { "🔴" },
        name, symbol,
        market_cap_display,
        if change_pct > 0.0 { "+" } else { "" },
        change_pct.abs())
} else {
    format!("{} {} ({})", Emojis::TOKEN, name, symbol)
};
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(title)
                    .url(format!("https://station.jup.ag/token/{}", price_data.address.as_deref().unwrap_or(symbol)))
                    .description(format!("**Token Details**\nPrice: {}\nVolume: ${:.2}M\nAge: {}\nSecurity: {}", 
                        price_display, 
                        daily_volume / 1_000_000.0,
                        age_text,
                        security_rating
                    ));
                    
                // Add logo if available
                if !logo_url.is_empty() {
                    embed = embed.thumbnail(logo_url);
                }
                
                // Add tags if available
                if !tags.is_empty() {
                    embed = embed.field("Tags", tags, false);
                }
                
                // Add links
                embed = embed.field("Links", explorer_links, false);
                
                // Add Hikari's comment
                let hikari_comment = if change_pct > 5.0 {
                    format!("{}! This token is pumping right now! Check the chart for breakouts!", 
                        HikariStyle::random_excited())
                } else if daily_volume > 1_000_000.0 {
                    "Seeing some good volume on this one - could be a trending token! Watch for a breakout fr fr! 👀".to_string()
                } else {
                    format!("Found this Solana token on Jupiter! {} Check it out if you're looking for the next gem!", 
                        HikariStyle::random_excited())
                };
                
                embed = embed.field(format!("{}Hikari's Take", Emojis::BOBA), hikari_comment, false);
                
                // Set color based on change
                embed.color(HikariStyle::get_trend_color(change_pct))
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending detailed token info: {}", e);
            return false;
        }
        
        true
    }
    
    // Fetch token data from Jupiter API
    async fn fetch_token_from_jupiter(&self, token_id: &str) -> Result<TokenData, Box<dyn StdError + Send + Sync>> {
        println!("Fetching token data from Jupiter API for: {}", token_id);
        
        // First fetch token metadata if possible
        let mut symbol = "UNKNOWN".to_string();
        let mut name = None;
        
        // Try to get token metadata from Jupiter token list
        let token_list_url = JUPITER_POPULAR_TOKENS_URL;
        let token_list_response = self.http_client
            .get(token_list_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        
        if let Ok(response) = token_list_response {
            if response.status().is_success() {
                if let Ok(tokens) = response.json::<Vec<serde_json::Value>>().await {
                    // Find the token in the list
                    for token in tokens {
                        if let Some(address) = token["address"].as_str() {
                            if address.to_lowercase() == token_id.to_lowercase() {
                                symbol = token["symbol"].as_str().unwrap_or("UNKNOWN").to_string();
                                name = token["name"].as_str().map(|s| s.to_string());
                                println!("Found token in Jupiter list: {} ({})", name.clone().unwrap_or_default(), symbol);
                                break;
                            }
                        }
                    }
                }
            }
        }
        
        // Now get price data
        let url = format!("{}/price?ids={}", JUPITER_PRICE_API, token_id);
        println!("Jupiter Price API URL: {}", url);
        
        let response = self.http_client
            .get(&url)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| {
                println!("Jupiter Price API request failed: {}", e);
                ScannerError::new(&format!("Jupiter Price API request failed: {}", e))
            })?;
        
        if !response.status().is_success() {
            println!("Jupiter Price API returned error: {}", response.status());
            return Err(Box::new(ScannerError::new(&format!(
                "Jupiter Price API error: {}", response.status()
            ))));
        }
        
        let data: serde_json::Value = response.json().await
            .map_err(|e| {
                println!("Failed to parse Jupiter response: {}", e);
                ScannerError::new(&format!("Failed to parse Jupiter response: {}", e))
            })?;
        
        println!("Jupiter API response parsed");
        
        // Check if data exists and contains the token
        if !data["data"].is_object() || !data["data"].get(token_id).map_or(false, |v| v.is_object()) {
            println!("Token {} not found in Jupiter price data", token_id);
            return Err(Box::new(ScannerError::new(&format!("Token {} not found in Jupiter API", token_id))));
        }
        
        let token_data = &data["data"][token_id];
        
        // Parse the price value - handling both string and number formats
        let price = if token_data["price"].is_string() {
            let price_str = token_data["price"].as_str().unwrap_or("0");
            price_str.parse::<f64>().unwrap_or(0.0)
        } else {
            token_data["price"].as_f64().unwrap_or(0.0)
        };
        
        // For Jupiter, we don't have all data fields, so we fill what we can
        let token = TokenData {
            symbol: symbol,
            name: name,
            price,
            price_change_24h: None, // Jupiter v4 doesn't provide this in the basic endpoint
            volume_24h: None,
            market_cap: None,
            fully_diluted_valuation: None,
            total_supply: None,
            max_supply: None,
            last_updated: Utc::now().to_string(),
            address: Some(token_id.to_string()),
            chain: "solana".to_string(),
        };
        
        println!("Successfully retrieved token data from Jupiter: {}", token.symbol);
        Ok(token)
    }
    
    // Fetch token data from CoinGecko API
    async fn fetch_token_from_coingecko(&self, token_id: &str) -> Result<TokenData, Box<dyn StdError + Send + Sync>> {
        println!("Fetching token data from CoinGecko API for: {}", token_id);
        
        // For Solana address, try to use contract address endpoint
        let url = if token_id.len() >= 32 && token_id.len() <= 44 {
            format!(
                "{}/coins/solana/contract/{}", 
                COINGECKO_API_URL, token_id
            )
        } else {
            // For regular coins like bitcoin, ethereum, etc.
            format!(
                "{}/coins/markets?vs_currency=usd&ids={}&order=market_cap_desc&per_page=1&page=1&sparkline=false&price_change_percentage=24h", 
                COINGECKO_API_URL, token_id
            )
        };
        
        println!("CoinGecko API URL: {}", url);
        
        let response = self.http_client
            .get(&url)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| {
                println!("CoinGecko API request failed: {}", e);
                ScannerError::new(&format!("CoinGecko API request failed: {}", e))
            })?;
        
        if !response.status().is_success() {
            println!("CoinGecko API returned error: {}", response.status());
            return Err(Box::new(ScannerError::new(&format!(
                "CoinGecko API error: {}", response.status()
            ))));
        }
        
        // Handle different response formats based on endpoint
        if token_id.len() >= 32 && token_id.len() <= 44 {
            // Contract address endpoint returns a single object
            let data: serde_json::Value = response.json().await
                .map_err(|e| {
                    println!("Failed to parse CoinGecko contract response: {}", e);
                    ScannerError::new(&format!("Failed to parse CoinGecko response: {}", e))
                })?;
            
            println!("CoinGecko contract API response parsed");
            
            // Extract token data from contract endpoint response
            let token = TokenData {
                symbol: data["symbol"].as_str().unwrap_or("Unknown").to_string().to_uppercase(),
                name: data["name"].as_str().map(|s| s.to_string()),
                price: data["market_data"]["current_price"]["usd"].as_f64().unwrap_or(0.0),
                price_change_24h: data["market_data"]["price_change_percentage_24h"].as_f64(),
                volume_24h: data["market_data"]["total_volume"]["usd"].as_f64(),
                market_cap: data["market_data"]["market_cap"]["usd"].as_f64(),
                fully_diluted_valuation: data["market_data"]["fully_diluted_valuation"]["usd"].as_f64(),
                total_supply: data["market_data"]["total_supply"].as_f64(),
                max_supply: data["market_data"]["max_supply"].as_f64(),
                last_updated: data["last_updated"].as_str().unwrap_or("").to_string(),
                address: Some(token_id.to_string()),
                chain: "solana".to_string(),
            };
            
            println!("Successfully retrieved token data from CoinGecko contract endpoint: {}", token.symbol);
            Ok(token)
        } else {
            // Markets endpoint returns an array
            let data: Vec<serde_json::Value> = response.json().await
                .map_err(|e| {
                    println!("Failed to parse CoinGecko markets response: {}", e);
                    ScannerError::new(&format!("Failed to parse CoinGecko response: {}", e))
                })?;
            
            println!("CoinGecko markets API response parsed");
            
            if data.is_empty() {
                println!("Token {} not found in CoinGecko", token_id);
                return Err(Box::new(ScannerError::new(&format!("Token {} not found in CoinGecko", token_id))));
            }
            
            let token_data = &data[0];
            
            // Extract chain info based on symbol if possible
            let chain = match token_data["symbol"].as_str().unwrap_or("").to_lowercase().as_str() {
                "sol" => "solana",
                "eth" => "ethereum",
                "matic" => "polygon",
                "bnb" => "binance-smart-chain",
                _ => "ethereum", // Default chain
            };
            
            let token = TokenData {
                symbol: token_data["symbol"].as_str().unwrap_or("Unknown").to_string().to_uppercase(),
                name: token_data["name"].as_str().map(|s| s.to_string()),
                price: token_data["current_price"].as_f64().unwrap_or(0.0),
                price_change_24h: token_data["price_change_percentage_24h"].as_f64(),
                volume_24h: token_data["total_volume"].as_f64(),
                market_cap: token_data["market_cap"].as_f64(),
                fully_diluted_valuation: token_data["fully_diluted_valuation"].as_f64(),
                total_supply: token_data["total_supply"].as_f64(),
                max_supply: token_data["max_supply"].as_f64(),
                last_updated: token_data["last_updated"].as_str().unwrap_or("").to_string(),
                address: None,
                chain: chain.to_string(),
            };
            
            println!("Successfully retrieved token data from CoinGecko markets endpoint: {}", token.symbol);
            Ok(token)
        }
    }
    
    // Update trending tokens from Jupiter API
    async fn update_trending_tokens(&self, force: bool) -> Result<(), Box<dyn StdError + Send + Sync>> {
        println!("Updating trending tokens{}...", if force { " (forced)" } else { "" });
        let mut last_update = self.last_trending_update.write().await;
        
        // Only check the time if not forcing an update
        if !force && last_update.elapsed().as_secs() < TRENDING_CACHE_SECONDS {
            println!("Skipping update - cache still valid");
            return Ok(());
        }
        
        *last_update = Instant::now();
        drop(last_update);
        
        // Try to fetch Jupiter trending tokens
        let jupiter_trending = match self.fetch_jupiter_trending().await {
            Ok(tokens) => {
                println!("Successfully fetched {} Jupiter trending tokens", tokens.len());
                tokens
            },
            Err(e) => {
                println!("Error fetching Jupiter trending tokens: {} - Details: {:?}", e, e);
                Vec::new()
            }
        };
        
        // Use Jupiter trending or fallback
        let mut all_trending = jupiter_trending;
        
        // Add debug printout for API testing
        println!("Debug: Attempting direct API requests for diagnostics");
        if let Ok(response) = self.http_client.get(format!("{}/popular-tokens", JUPITER_PRICE_API)).send().await {
            if let Ok(text) = response.text().await {
                let preview_length = std::cmp::min(200, text.len());
                println!("Debug - Jupiter API raw response: {}", &text[..preview_length]);
            }
        }
        
        // Only sort if we have tokens
        if !all_trending.is_empty() {
            // Sort by volume but with fallback for NaN or missing values
            all_trending.sort_by(|a, b| {
                // First try to compare by volume
                match b.volume_24h.partial_cmp(&a.volume_24h) {
                    Some(ordering) => ordering,
                    None => {
                        // If volume comparison fails, try market cap
                        match (b.market_cap, a.market_cap) {
                            (Some(b_cap), Some(a_cap)) => b_cap.partial_cmp(&a_cap).unwrap_or(std::cmp::Ordering::Equal),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => {
                                // Last resort, compare by price
                                b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal)
                            }
                        }
                    }
                }
            });
            
            // Update cache
            let mut trending_cache = self.trending_cache.write().await;
            *trending_cache = all_trending;
            
            println!("Updated trending cache with {} tokens", trending_cache.len());
        } else {
            println!("No trending tokens found from any source - using fallback data");
            
            // If we couldn't get data from the API, use a fallback
            let fallback_tokens = self.get_fallback_trending_tokens().await;
            
            // Update the cache with fallback tokens
            let mut trending_cache = self.trending_cache.write().await;
            *trending_cache = fallback_tokens;
            
            println!("Updated trending cache with fallback tokens");
        }
        
        Ok(())
    }
    
    // Fetch trending tokens from Jupiter
    async fn fetch_jupiter_trending(&self) -> Result<Vec<TrendingToken>, Box<dyn StdError + Send + Sync>> {
        println!("Fetching trending tokens from Jupiter...");
        
        // Use the Jupiter strict token list endpoint
        let url = JUPITER_POPULAR_TOKENS_URL;
        println!("Jupiter trending API URL: {}", url);
        
        let response = self.http_client
            .get(url)
            .timeout(Duration::from_secs(15))
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                println!("Jupiter trending API request failed: {}", e);
                ScannerError::new(&format!("Jupiter trending API request failed: {}", e))
            })?;
        
        // Check status
        let status = response.status();
        if !status.is_success() {
            println!("Jupiter trending API returned error: {}", status);
            return Err(Box::new(ScannerError::new(&format!(
                "Jupiter trending API error: {}", status
            ))));
        }
        
        // Parse the JSON response directly
        let tokens: Vec<serde_json::Value> = response.json().await
            .map_err(|e| {
                println!("Failed to parse Jupiter token list: {}", e);
                ScannerError::new(&format!("Failed to parse Jupiter token list: {}", e))
            })?;
        
        println!("Jupiter token list parsed, found {} tokens", tokens.len());
        
        // Sort by daily volume if available, then take top 10
        let mut sorted_tokens = tokens.clone();
        sorted_tokens.sort_by(|a, b| {
            let vol_a = a["daily_volume"].as_f64().unwrap_or(0.0);
            let vol_b = b["daily_volume"].as_f64().unwrap_or(0.0);
            vol_b.partial_cmp(&vol_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        // For trending tokens, we'll take the first 10 tokens with highest volume
        let trending_tokens = sorted_tokens.into_iter().take(15).collect::<Vec<_>>();
        println!("Selected top {} trending tokens by volume", trending_tokens.len());
        
        // Now we need to get price info for each token
        let mut trending = Vec::new();
        
        // Get price data for each token in batches to avoid hitting rate limits
        let token_addresses = trending_tokens.iter()
            .map(|t| t["address"].as_str().unwrap_or("").to_string())
            .filter(|addr| !addr.is_empty())
            .collect::<Vec<_>>();
        
        // Join addresses with comma for batch request
        let addresses_str = token_addresses.join(",");
        
        // Get price data for all tokens in one request
        if !addresses_str.is_empty() {
            let price_url = format!("{}?ids={}", JUPITER_PRICE_API, addresses_str);
            println!("Fetching price data for trending tokens: {}", price_url);
            
            let price_response = match self.http_client
                .get(&price_url)
                .timeout(Duration::from_secs(15))
                .send()
                .await {
                    Ok(response) => {
                        if response.status().is_success() {
                            match response.json::<serde_json::Value>().await {
                                Ok(data) => Some(data),
                                Err(e) => {
                                    println!("Error parsing price response: {}", e);
                                    None
                                }
                            }
                        } else {
                            println!("Price API returned error: {}", response.status());
                            None
                        }
                    },
                    Err(e) => {
                        println!("Error fetching price data: {}", e);
                        None
                    }
                };
            
            // Process tokens with price data
            for token in trending_tokens.iter() {
                let address = token["address"].as_str().unwrap_or("unknown").to_string();
                let symbol = token["symbol"].as_str().unwrap_or("UNKNOWN").to_string();
                let name = token["name"].as_str().unwrap_or(&symbol).to_string();
                let volume = token["daily_volume"].as_f64().unwrap_or(0.0);
                
                // Get price and price change from price response if available
                let (price, price_change) = if let Some(price_data) = &price_response {
                    if let Some(token_price) = price_data["data"].get(&address) {
                        // Extract price data from response
                        let price = token_price["price"].as_str()
                            .and_then(|p| p.parse::<f64>().ok())
                            .or_else(|| token_price["price"].as_f64())
                            .unwrap_or(0.0);
                        
                        // Price change might not be available
                        let price_change = 0.0; // Default for now, could be improved with historical data
                        
                        (price, price_change)
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    (0.0, 0.0)
                };
                
                trending.push(TrendingToken {
                    address,
                    symbol: symbol.clone(),
                    name: name.clone(),
                    price,
                    price_change_24h: price_change,
                    volume_24h: volume,
                    created_at: None,
                    chain: "solana".to_string(),
                    market_cap: None, // Not available in this endpoint
                });
                
                println!("Added Jupiter token: {} ({}) - price: {}", name, symbol, price);
            }
        }
        
        // If we couldn't get any trending tokens with prices, return empty list
        if trending.is_empty() {
            println!("No trending tokens with price data found");
        }
        
        // Sort by volume
        trending.sort_by(|a, b| {
            b.volume_24h.partial_cmp(&a.volume_24h).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        Ok(trending)
    }
    
    // Handle command parsing and routing
    async fn handle_commands(&self, ctx: &Context, msg: &Message) -> bool {
        let content = msg.content.to_lowercase();
        let parts: Vec<&str> = content.split_whitespace().collect();
        
        if parts.is_empty() {
            return false;
        }
        
        println!("Processing command: {}", parts[0]);
        
        match parts[0] {
            "!sol" | "!solana" => {
                if parts.len() < 2 {
                    return self.send_solana_info(ctx, msg.channel_id).await;
                }
                
                match parts[1] {
                    "trending" => self.send_trending_tokens(ctx, msg.channel_id).await,
                    "price" if parts.len() > 2 => self.send_token_price(ctx, msg.channel_id, parts[2]).await,
                    "address" if parts.len() > 2 => self.send_solana_address_info(ctx, msg.channel_id, parts[2]).await,
                    _ => self.send_help_message(ctx, msg.channel_id).await,
                }
            },
            "!price" => {
                if parts.len() > 1 {
                    self.send_token_price(ctx, msg.channel_id, parts[1]).await
                } else {
                    self.send_help_message(ctx, msg.channel_id).await
                }
            },
            "!wallet" => {
                if parts.len() > 1 {
                    self.send_wallet_info(ctx, msg.channel_id, parts[1]).await
                } else {
                    self.send_help_message(ctx, msg.channel_id).await
                }
            },
            "!nft" => {
                if parts.len() > 1 {
                    self.send_nft_info(ctx, msg.channel_id, parts[1]).await
                } else {
                    self.send_help_message(ctx, msg.channel_id).await
                }
            },
            "!trending" => self.send_trending_tokens(ctx, msg.channel_id).await,
            _ => false,
        }
    }
    
    // Handle direct coin mentions
    async fn handle_coin_mention(&self, ctx: &Context, msg: &Message) -> bool {
        let content = msg.content.to_lowercase();
        
        // Extract potential coin name/symbol
        if content.contains("bitcoin") || content.contains("btc") {
            return self.send_token_price(ctx, msg.channel_id, "bitcoin").await;
        } else if content.contains("ethereum") || content.contains("eth") {
            return self.send_token_price(ctx, msg.channel_id, "ethereum").await;
        } else if content.contains("solana") || content.contains("sol") {
            return self.send_token_price(ctx, msg.channel_id, "solana").await;
        }
        
        false
    }
    
    // Send basic Solana info
    async fn send_solana_info(&self, ctx: &Context, channel_id: ChannelId) -> bool {
        println!("Sending Solana info");
        
        match self.get_token_price("solana").await {
            Ok(solana) => {
                if let Err(e) = channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title(format!("{} Solana (SOL) Info", Emojis::SOLANA))
                            .description(format!("{}Slick, high-performance L1 blockchain with lightning-fast transactions and low fees fr fr!{}", 
                                                Emojis::INFO, Emojis::INFO))
                            .field(format!("{}Price", Emojis::PRICE), 
                                  format!("${:.4}", solana.price), true)
                            .field(format!("{}24h Change", Emojis::CHANGE), 
                                  format!("{} {:.2}%", 
                                         HikariStyle::get_trend_emoji(solana.price_change_24h.unwrap_or(0.0)),
                                         solana.price_change_24h.unwrap_or(0.0)), 
                                  true)
                            .field(format!("{}24h Volume", Emojis::VOLUME), 
                                  format!("${:.2}M", solana.volume_24h.unwrap_or(0.0) / 1_000_000.0), 
                                  true)
                            .field(format!("{}Market Cap", Emojis::MARKET_CAP), 
                                  format!("${:.2}B", solana.market_cap.unwrap_or(0.0) / 1_000_000_000.0), 
                                  true)
                            .field(format!("{}Features", Emojis::INFO),
                                  "• 65,000 TPS capability\n• <$0.001 transaction fees\n• 400ms block times\n• Energy efficient", 
                                  false)
                            .field(format!("{}Hikari's Take", Emojis::BOBA),
                                  format!("{}! Solana's my fave L1, perfect for DeFi and NFTs with insane speed and super cheap gas!",
                                         HikariStyle::random_excited()),
                                  false)
                            .color(HikariStyle::get_trend_color(solana.price_change_24h.unwrap_or(0.0)))
                            .thumbnail("https://cryptologos.cc/logos/solana-sol-logo.png")
                            .footer(|f| HikariStyle::apply_footer(f))
                    })
                }).await {
                    println!("Error sending Solana info: {}", e);
                    return false;
                }
                println!("Solana info sent successfully");
                true
            },
            Err(e) => {
                println!("Error getting Solana price: {}", e);
                if let Err(err) = channel_id.say(&ctx.http, 
                    format!("{} Couldn't get Solana info rn... blockchain be glitchin! Try again later bestie!", Emojis::ERROR)).await {
                    println!("Error sending error message: {}", err);
                }
                false
            }
        }
    }
    
    // Send Solana address info
    async fn send_solana_address_info(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        println!("Sending Solana address info for: {}", address);
        
        // Use multiple sources to determine address type
        // First try Solscan API
        let response = self.http_client
            .get(format!("{}/account/{}", SOLSCAN_API_URL, address))
            .header("accept", "application/json")
            .send()
            .await;
                
        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    // Successful response, try to parse
                    match resp.json::<serde_json::Value>().await {
                        Ok(info) => {
                            let account_type = info["type"].as_str().unwrap_or("Unknown");
                            println!("Address type from Solscan: {}", account_type);
                            
                            let result = match account_type {
                                "token_account" => self.send_token_account_info(ctx, channel_id, &info, address).await,
                                "nft" => self.send_nft_account_info(ctx, channel_id, &info, address).await,
                                _ => self.send_regular_account_info(ctx, channel_id, &info, address).await,
                            };
                            
                            if result {
                                println!("Solana address info sent successfully");
                                return true;
                            }
                        },
                        Err(e) => {
                            println!("Error parsing account data from Solscan: {}", e);
                            // Continue to fallback methods
                        }
                    }
                } else if resp.status().as_u16() == 404 {
                    println!("Address not found on Solscan, trying fallback methods");
                    // Continue to fallback methods
                } else {
                    println!("Solscan API error: {}", resp.status());
                    // Continue to fallback methods
                }
            },
            Err(e) => {
                println!("Error connecting to Solscan API: {}", e);
                // Continue to fallback methods
            }
        }
        
        // Fallback: Try to get token info from Jupiter
        let token_response = self.check_token_on_jupiter(address).await;
        if let Some(token_info) = token_response {
            return self.send_jupiter_token_info(ctx, channel_id, &token_info, address).await;
        }
        
        // Final fallback: Send enhanced wallet info
        self.send_enhanced_wallet_info(ctx, channel_id, address).await
    }
    
    // Send enhanced wallet info
    async fn send_enhanced_wallet_info(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        println!("Sending enhanced wallet info for: {}", address);
        
        // We'll create a more informative display with additional explorer links
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Solana Wallet", Emojis::WALLET))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .description(format!("Address: `{}`\n\nCheck out this Solana wallet on the explorers below.", address))
                    .field("Explorer Links", 
                           format!("→ [Solana Explorer]({}/address/{})\n→ [Solscan](https://solscan.io/account/{})\n→ [SolanaFM](https://solana.fm/address/{})\n→ [Solflare](https://solflare.network/address/{})\n→ [Xray](https://xray.helius.xyz/account/{})", 
                                  EXPLORER_URL, address, address, address, address, address), 
                           false)
                    .field("Token Holdings", 
                           format!("→ [View on Jito](https://explorer.jito.wtf/address/{})\n→ [View on Jupiter](https://station.jup.ag/profile/{})", 
                                   address, address), 
                           false)
                    .field(format!("{}Hikari's Take", Emojis::BOBA),
                          "This looks like a Solana wallet! I can't get all the on-chain deets rn, but check out these explorers to see their SOL balance, tokens, and NFTs! Probably a based trader fr fr!",
                          false)
                    .color(Color::from_rgb(20, 241, 149)) // Solana green  
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending enhanced wallet info: {}", e);
            return false;
        }
        
        true
    }
    
    // Try to get token info from Jupiter
    async fn check_token_on_jupiter(&self, address: &str) -> Option<serde_json::Value> {
        let url = format!("{}?ids={}", JUPITER_PRICE_API, address);
        
        if let Ok(response) = self.http_client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await {
            
            if response.status().is_success() {
                if let Ok(data) = response.json::<serde_json::Value>().await {
                    if data["data"].is_object() && data["data"].get(address).is_some() {
                        // Price data found, now try to get token metadata
                        let token_list_url = JUPITER_POPULAR_TOKENS_URL;
                        
                        if let Ok(list_response) = self.http_client
                            .get(token_list_url)
                            .timeout(Duration::from_secs(10))
                            .send()
                            .await {
                            
                            if list_response.status().is_success() {
                                if let Ok(tokens) = list_response.json::<Vec<serde_json::Value>>().await {
                                    // Find the token in the list
                                    for token in tokens {
                                        if let Some(token_address) = token["address"].as_str() {
                                            if token_address.to_lowercase() == address.to_lowercase() {
                                                let mut token_info = token.clone();
                                                // Add price data
                                                if let Some(price_data) = data["data"].get(address) {
                                                    token_info["price"] = price_data["price"].clone();
                                                }
                                                return Some(token_info);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        
                        // If we couldn't find metadata but have price, create basic token info
                        let mut basic_info = serde_json::json!({
                            "address": address,
                            "symbol": "UNKNOWN", 
                            "name": "Unknown Token",
                        });
                        
                        // Add price data
                        if let Some(price_data) = data["data"].get(address) {
                            basic_info["price"] = price_data["price"].clone();
                        }
                        
                        return Some(basic_info);
                    }
                }
            }
        }
        
        None
    }
    
    // Get account info from Explorer API
    async fn get_account_from_explorer(&self, address: &str) -> Result<serde_json::Value, Box<dyn StdError + Send + Sync>> {
        let mut result = serde_json::json!({
            "address": address,
            "type": "Unknown",
            "balance": 0,
        });
        
        // This is a simple implementation since we don't have direct access to Explorer API
        // In a real implementation, you would use the Explorer API directly
        
        Ok(result)
    }
    
    // Show Jupiter token info
    async fn send_jupiter_token_info(&self, ctx: &Context, channel_id: ChannelId, token_info: &serde_json::Value, address: &str) -> bool {
        // Try to get detailed info first
        if let Some(token_details) = self.get_token_details(address).await {
            // We have full details, use the comprehensive display
            let price_data = TokenData {
                symbol: token_details["symbol"].as_str().unwrap_or("UNKNOWN").to_string(),
                name: token_details["name"].as_str().map(|n| n.to_string()),
                price: if token_info["price"].is_string() {
                    token_info["price"].as_str()
                        .and_then(|p| p.parse::<f64>().ok())
                        .unwrap_or(0.0)
                } else {
                    token_info["price"].as_f64().unwrap_or(0.0)
                },
                price_change_24h: None, // Not available from this API
                volume_24h: token_details["daily_volume"].as_f64(),
                market_cap: None,
                fully_diluted_valuation: None,
                total_supply: None,
                max_supply: None,
                last_updated: Utc::now().to_string(),
                address: Some(address.to_string()),
                chain: "solana".to_string(),
            };
            
            return self.send_detailed_token_info(ctx, channel_id, &token_details, &price_data).await;
        }
        
        // Fallback to the old method if we couldn't get detailed info
        let symbol = token_info["symbol"].as_str().unwrap_or("UNKNOWN");
        let name = token_info["name"].as_str().unwrap_or(symbol);
        
        // Get price from token info
        let price = if token_info["price"].is_string() {
            token_info["price"].as_str()
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(0.0)
        } else {
            token_info["price"].as_f64().unwrap_or(0.0)
        };
        
        // Get volume if available
        let volume = token_info["daily_volume"].as_f64().unwrap_or(0.0);
        
        // Send token info embed
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{} {} Token ({}) Info", Emojis::SOLANA, name, symbol))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .description(format!("{}**Token Info**\nThis address is a Solana token mint. Check it out on [Jupiter](https://station.jup.ag/token/{})", Emojis::INFO, address));
                
                // Add price info
                embed = embed.field("Price", format!("${:.8}", price), true);
                
                // Add volume if available
                if volume > 0.0 {
                    if volume >= 1_000_000.0 {
                        embed = embed.field("24h Volume", format!("${:.2}M", volume / 1_000_000.0), true);
                    } else {
                        embed = embed.field("24h Volume", format!("${:.2}K", volume / 1_000.0), true);
                    }
                }
                
                // Add logo if available
                if let Some(logo_url) = token_info["logoURI"].as_str() {
                    embed = embed.thumbnail(logo_url);
                }
                
                // Add Hikari's comment
                embed = embed.field(format!("{}Hikari's Take", Emojis::BOBA),
                    format!("{}! This looks like a legit Solana token with good liquidity on Jupiter!", 
                        HikariStyle::random_excited()),
                    false);
                
                embed.color(Color::from_rgb(114, 9, 183)) // Purple
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending Jupiter token info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send generic address info with links to explorers
    async fn send_generic_address_info(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Solana Address", Emojis::SOLANA))
                    .description(format!("Address: `{}`\n\nThis address has been found on the Solana blockchain, but I couldn't retrieve detailed information about it.", address))
                    .field("View on Explorer", 
                           format!("[Solana Explorer]({}/address/{})\n[Solscan](https://solscan.io/account/{})\n[SolanaFM](https://solana.fm/address/{})", 
                                  EXPLORER_URL, address, address, address), 
                           false)
                    .field(format!("{}Hikari's Take", Emojis::BOBA),
                          "Can't tell exactly what this address is, but you can check it out on one of the explorers above for more details!",
                          false)
                    .color(Color::from_rgb(20, 241, 149)) // Solana green
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending generic address info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send simplified account info
    async fn send_simple_account_info(&self, ctx: &Context, channel_id: ChannelId, info: &serde_json::Value, address: &str) -> bool {
        // This is a fallback if the Solscan API fails
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Solana Address", Emojis::WALLET))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .description(format!("Address: `{}`\n\nView more details on [Solana Explorer]({}/address/{}) or [Solscan](https://solscan.io/account/{})", 
                                        address, EXPLORER_URL, address, address))
                    .field(format!("{}Hikari's Take", Emojis::BOBA),
                          "I found this address on Solana but couldn't get all the details. Check it out on one of the explorers for more info!",
                          false)
                    .color(Color::from_rgb(20, 241, 149)) // Solana green  
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending simple account info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send token account info
    async fn send_token_account_info(&self, ctx: &Context, channel_id: ChannelId, info: &serde_json::Value, address: &str) -> bool {
        println!("Sending token account info");
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{} Solana Token Account", Emojis::TOKEN))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .color(Color::from_rgb(149, 0, 255)); // Solana purple
                
                if let Some(token_info) = info["tokenInfo"].as_object() {
                    let symbol = token_info["symbol"].as_str().unwrap_or("Unknown");
                    let token_name = token_info["name"].as_str().unwrap_or("Unknown Token");
                    let token_address = token_info["tokenMint"].as_str().unwrap_or("");
                    
                    let amount = info["tokenAmount"]["uiAmount"].as_f64().unwrap_or(0.0);
                    
                    embed = embed
                        .description(format!("{}**Token Account**\nHolds {} {}", Emojis::INFO, amount, symbol))
                        .field("Token", symbol, true)
                        .field("Name", token_name, true)
                        .field("Balance", format!("{:.4}", amount), true)
                        .field("Token Address", 
                               format!("[{}...{}]({}/address/{})", 
                                      &token_address[0..6], 
                                      &token_address[token_address.len().saturating_sub(4)..],
                                      EXPLORER_URL,
                                      token_address), 
                               false);
                }
                
                if let Some(owner) = info["owner"].as_str() {
                    embed = embed.field("Owner", 
                                       format!("[{}...{}]({}/address/{})", 
                                              &owner[0..6], 
                                              &owner[owner.len().saturating_sub(4)..],
                                              EXPLORER_URL,
                                              owner), 
                                       true);
                }
                
                embed.footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending token account info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send NFT account info
    async fn send_nft_account_info(&self, ctx: &Context, channel_id: ChannelId, info: &serde_json::Value, address: &str) -> bool {
        println!("Sending NFT account info");
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{} Solana NFT", Emojis::NFT))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .color(Color::from_rgb(113, 62, 194)); // Solana NFT purple
                
                embed = embed.description(format!("{}**NFT Token**", Emojis::INFO));
                
                if let Some(collection) = info["collection"]["name"].as_str() {
                    embed = embed.field("Collection", collection, true);
                }
                
                if let Some(name) = info["metadata"]["name"].as_str() {
                    embed = embed.field("Name", name, true);
                }
                
                if let Some(image_url) = info["metadata"]["image"].as_str() {
                    embed = embed.thumbnail(image_url);
                }
                
                if let Some(owner) = info["owner"].as_str() {
                    embed = embed.field("Owner", 
                                       format!("[{}...{}]({}/address/{})", 
                                              &owner[0..6], 
                                              &owner[owner.len().saturating_sub(4)..],
                                              EXPLORER_URL,
                                              owner), 
                                       true);
                }
                
                embed.footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending NFT account info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send regular account info
    async fn send_regular_account_info(&self, ctx: &Context, channel_id: ChannelId, info: &serde_json::Value, address: &str) -> bool {
        println!("Sending regular account info");
        
        let lamports = info["lamports"].as_f64().unwrap_or(0.0) / 1_000_000_000.0; // Convert to SOL
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{} Solana Account", Emojis::WALLET))
                    .url(format!("{}/address/{}", EXPLORER_URL, address))
                    .color(Color::from_rgb(20, 241, 149)); // Solana green
                
                let account_type = info["type"].as_str().unwrap_or("Unknown");
                
                embed = embed
                    .description(format!("{}**{} Account**", Emojis::INFO, account_type))
                    .field("SOL Balance", format!("{:.6} {}", lamports, Emojis::SOL), true);
                    
                if let Some(owner) = info["owner"].as_str() {
                    embed = embed.field("Owner", 
                                       format!("[{}...{}]({}/address/{})", 
                                              &owner[0..6], 
                                              &owner[owner.len().saturating_sub(4)..],
                                              EXPLORER_URL,
                                              owner), 
                                       true);
                }
                
                if info["executable"].as_bool().unwrap_or(false) {
                    embed = embed.field("Executable", "✅ This is a program", false);
                }
                
                embed.footer(|f| f.text(format!("Address: {} • Powered by Hikari 🧋", address)))
            })
        }).await {
            println!("Error sending account info: {}", e);
            return false;
        }
        
        true
    }
    
    // Send Ethereum address info (simplified implementation)
    async fn send_eth_address_info(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        println!("Sending ETH address info for: {}", address);
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Ethereum Address Detected", Emojis::ETH))
                    .description(format!("Ethereum address: `{}`\n\nNot gonna lie bestie, I'm better with Solana than ETH! 🧋 Here's what I know about this address though...", address))
                    .field("View on Explorer", 
                           format!("[Etherscan](https://etherscan.io/address/{})", address), 
                           false)
                    .field("Hikari's Take", 
                           "ETH addresses work, but you know what's even better? Solana! Try checking a SOL address next time for way more info!", 
                           false)
                    .url(format!("https://etherscan.io/address/{}", address))
                    .color(Color::from_rgb(114, 137, 218)) // Discord blue
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending ETH address info: {}", e);
            return false;
        }
        
        println!("ETH address info sent successfully");
        true
    }
    
    // Send wallet info (may contain multiple tokens)
    async fn send_wallet_info(&self, ctx: &Context, channel_id: ChannelId, address: &str) -> bool {
        // Currently just a wrapper for send_solana_address_info
        self.send_solana_address_info(ctx, channel_id, address).await
    }
    
    // Send NFT collection/item info
    async fn send_nft_info(&self, ctx: &Context, channel_id: ChannelId, nft_id: &str) -> bool {
        println!("Sending NFT info placeholder for: {}", nft_id);
        
        if let Err(e) = channel_id.say(&ctx.http, 
            format!("{}NFT info feature coming soon! For now, try checking the collection address directly!", Emojis::NFT)).await {
            println!("Error sending NFT info placeholder: {}", e);
            return false;
        }
        
        true
    }
    
    // Send trending tokens list
    async fn send_trending_tokens(&self, ctx: &Context, channel_id: ChannelId) -> bool {
        println!("Sending trending tokens");
        
        // Check if cache is empty and try to update if needed
        {
            let trending_cache = self.trending_cache.read().await;
            if trending_cache.is_empty() {
                drop(trending_cache); // Release the read lock before update
                println!("Trending cache is empty, forcing update");
                if let Err(e) = self.update_trending_tokens(true).await {
                    println!("Error updating trending tokens: {}", e);
                }
            }
        }
        
        // Now get the trending tokens from cache
        let trending = self.trending_cache.read().await.clone();
        
        if trending.is_empty() {
            println!("No trending data available after update attempt");
            if let Err(e) = channel_id.say(&ctx.http, 
                format!("{}No trending data available rn! Try again later, bestie!", Emojis::ERROR)).await {
                println!("Error sending error message: {}", e);
            }
            return false;
        }
        
        println!("Found {} trending tokens to display", trending.len());
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                let mut embed = e.title(format!("{}🔥 Trending Tokens on Solana", Emojis::HOT))
                    .description("Top tokens by volume in the last 24 hours... these bad boys are PUMPING rn! 👀")
                    .color(Color::from_rgb(255, 100, 0)); // Hot orange
                
                for (i, token) in trending.iter().take(10).enumerate() {
                    let price_emoji = if token.price_change_24h >= 0.0 { Emojis::UP } else { Emojis::DOWN };
                    let price_color = if token.price_change_24h >= 0.0 { "🟢" } else { "🔴" };
                    
                    let description = if token.volume_24h >= 1_000_000.0 {
                        format!(
                            "Price: ${:.6}\n{} {:.2}%\nVol: ${:.2}M\n[View]({}/address/{})",
                            token.price,
                            price_emoji,
                            token.price_change_24h.abs(),
                            token.volume_24h / 1_000_000.0,
                            EXPLORER_URL,
                            token.address
                        )
                    } else {
                        format!(
                            "Price: ${:.6}\n{} {:.2}%\nVol: ${:.2}K\n[View]({}/address/{})",
                            token.price,
                            price_emoji,
                            token.price_change_24h.abs(),
                            token.volume_24h / 1_000.0,
                            EXPLORER_URL,
                            token.address
                        )
                    };
                    
                    embed = embed.field(
                        format!("#{} {} {}", i+1, price_color, token.symbol),
                        description,
                        true
                    );
                }
                
                embed.footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending trending tokens: {}", e);
            return false;
        }
        
        println!("Trending tokens sent successfully");
        true
    }

    // Get fallback trending coins
    async fn get_fallback_trending_tokens(&self) -> Vec<TrendingToken> {
        println!("Using fallback trending tokens since APIs are unavailable");
        
        // Return some hardcoded tokens as fallback data
        vec![
            TrendingToken {
                address: "So11111111111111111111111111111111111111112".to_string(),  // SOL token
                symbol: "SOL".to_string(),
                name: "Solana".to_string(),
                price: 152.35,
                price_change_24h: 4.2,
                volume_24h: 1500000000.0,
                created_at: None,
                chain: "solana".to_string(),
                market_cap: Some(65000000000.0),
            },
            TrendingToken {
                address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),  // USDC on Solana
                symbol: "USDC".to_string(), 
                name: "USD Coin".to_string(),
                price: 1.0,
                price_change_24h: 0.01,
                volume_24h: 2500000000.0,
                created_at: None,
                chain: "solana".to_string(),
                market_cap: Some(28500000000.0),
            },
            TrendingToken {
                address: "7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj".to_string(),
                symbol: "BONK".to_string(),
                name: "Bonk".to_string(),
                price: 0.00003452,
                price_change_24h: 12.5,
                volume_24h: 85000000.0,
                created_at: None,
                chain: "solana".to_string(),
                market_cap: Some(2100000000.0),
            },
            // Add a few more tokens for variety
            TrendingToken {
                address: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                price: 1.0,
                price_change_24h: -0.02,
                volume_24h: 2100000000.0,
                created_at: None,
                chain: "solana".to_string(),
                market_cap: Some(97000000000.0),
            },
            TrendingToken {
                address: "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs".to_string(),
                symbol: "JUP".to_string(),
                name: "Jupiter".to_string(),
                price: 2.15,
                price_change_24h: 8.7,
                volume_24h: 45000000.0,
                created_at: None,
                chain: "solana".to_string(),
                market_cap: Some(850000000.0),
            }
        ]
    }    
    
    // Send help information about available commands
    async fn send_help_message(&self, ctx: &Context, channel_id: ChannelId) -> bool {
        println!("Sending help message");
        
        if let Err(e) = channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title("🧋 Hikari's Crypto Commands")
                    .description("Hey bestie! Here are all the things I can help you with:")
                    .field("Token Commands", 
                           "• `!price <symbol/name>` - Get token price\n\
                            • `!sol price <symbol/name>` - Get Solana token price\n\
                            • `!trending` - Show trending tokens\n\
                            • `!sol trending` - Show trending Solana tokens",
                           false)
                    .field("Address/Wallet Commands",
                           "• `!sol address <address>` - View Solana address info\n\
                            • `!wallet <address>` - View wallet holdings\n\
                            • Just paste any Solana address and I'll show you info!",
                           false)
                    .field("NFT Commands",
                           "• `!nft <collection/id>` - View NFT info (coming soon!)",
                           false)
                    .field("Pro Tip", 
                           "💎 I automatically detect Solana and ETH addresses when you paste them! No command needed!",
                           false)
                    .color(Color::from_rgb(114, 137, 218)) // Discord blue
                    .footer(|f| HikariStyle::apply_footer(f))
            })
        }).await {
            println!("Error sending help message: {}", e);
            return false;
        }
        
        println!("Help message sent successfully");
        true
    }
}

// Trait to make integration with waifu bot easier
#[async_trait]
pub trait HikariScannerHandler: Send + Sync {
    async fn handle_scan_events(&self, ctx: Context, msg: Message) -> bool;
}

#[async_trait]
impl HikariScannerHandler for HikariScanner {
    async fn handle_scan_events(&self, ctx: Context, msg: Message) -> bool {
        self.handle_message(ctx, msg).await
    }
}

// Create and initialize the scanner
pub fn create_scanner() -> Arc<HikariScanner> {
    println!("Creating HikariScanner instance");
    
    // Create API key map (can be populated later if needed)
    let mut api_keys = HashMap::new();
    
    // Create scanner instance
    let scanner = HikariScanner::new(api_keys);
    
    // Wrap scanner in Arc for thread-safe sharing
    let scanner_arc = Arc::new(scanner);
    
    // Trigger initial trending token update (in the background)
    let scanner_clone = Arc::clone(&scanner_arc);
    tokio::spawn(async move {
        println!("Performing initial trending tokens update...");
        if let Err(e) = scanner_clone.update_trending_tokens(false).await {
            println!("Error during initial trending tokens update: {}", e);
        } else {
            println!("Initial trending tokens update completed successfully");
        }
    });
    
    println!("HikariScanner created successfully");
    scanner_arc
}