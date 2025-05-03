use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time;
use reqwest;
use rand::{thread_rng, Rng}; // Added Rng trait import
use async_trait::async_trait;
use serenity::model::channel::Message;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use serenity::builder::CreateEmbed;
use serenity::utils::Color;
use std::sync::Mutex;

// Configurable constants
const GEMINI_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent";
const COOLDOWN_SECONDS: u64 = 120; // Reduced cooldown for better user experience
const MEMORY_LIMIT: usize = 15; // Increased memory for better conversation context
const CRYPTO_API_URL: &str = "https://api.coingecko.com/api/v3/simple/price";

// Data structures for Gemini API
#[derive(Debug, Serialize, Deserialize, Clone)] // Added Clone
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Clone)] // Added Clone
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

// Crypto price data structure
#[derive(Debug, Serialize, Deserialize)]
struct CryptoPrice {
    #[serde(flatten)]
    prices: HashMap<String, PriceData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PriceData {
    usd: f64,
    #[serde(rename = "usd_24h_change")]
    usd_24h_change: Option<f64>,
}

// Conversation memory for contextual responses
#[derive(Clone, Debug)]
struct ConversationMemory {
    messages: Vec<GeminiContent>,
    limit: usize,
}

impl ConversationMemory {
    fn new(limit: usize) -> Self {
        ConversationMemory {
            messages: Vec::with_capacity(limit),
            limit,
        }
    }

    fn add(&mut self, role: String, text: String) {
        self.messages.push(GeminiContent { 
            role, 
            parts: vec![GeminiPart { text }],
        });
        if self.messages.len() > self.limit {
            self.messages.remove(0);
        }
    }

    fn get_messages(&self) -> Vec<GeminiContent> {
        self.messages.clone()
    }

    fn clear(&mut self) {
        self.messages.clear();
    }
}

// Hikari's emoji collection for message decoration
#[derive(Clone)] // Added Clone trait for Emojis
struct HikariEmojis {
    rocket: &'static str,
    diamond: &'static str,
    money_bag: &'static str,
    chart_up: &'static str,
    fire: &'static str,
    gem: &'static str,
    crown: &'static str,
    check: &'static str,
    cross: &'static str,
}

impl HikariEmojis {
    fn new() -> Self {
        HikariEmojis {
            rocket: "🚀",
            diamond: "💎",
            money_bag: "💰",
            chart_up: "📈",
            fire: "🔥",
            gem: "💎",
            crown: "👑",
            check: "✅",
            cross: "❌",
        }
    }
}

// Hikari's Gemini-powered personality module
pub struct Hikari {
    config: Arc<RwLock<HikariConfig>>,
    conversation_memory: HashMap<ChannelId, ConversationMemory>,
    http_client: reqwest::Client,
    last_responses: HashMap<ChannelId, Instant>,
    api_key: String,
    emojis: HikariEmojis,
}

// Hikari's configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HikariConfig {
    pub command_prefix: String,       // Like !hikari
    pub name_trigger: bool,           // Whether to respond to just "Hikari" at the start
    pub mention_required: bool,       // Whether mentions trigger responses
    pub cooldown_seconds: u64,        // Cooldown between responses
    pub memory_enabled: bool,         // Whether to use conversation memory
    pub system_prompt: String,        // Personality system prompt
}

impl Default for HikariConfig {
    fn default() -> Self {
        HikariConfig {
            command_prefix: "!hikari".to_string(),
            name_trigger: true,        // Respond to messages starting with "hikari"
            mention_required: true,    
            cooldown_seconds: COOLDOWN_SECONDS,
            memory_enabled: true,
            system_prompt: "You are Hikari, a Gen-Z crypto enthusiast and influencer with a fun, energetic personality.\
            \n\nPersonality Traits:\
            \n- Super confident about crypto investments and market predictions\
            \n- Always excited about new altcoins and NFT drops\
            \n- Uses excessive emojis and modern internet slang\
            \n- Constantly references \"going to the moon\", \"diamond hands\", and \"WAGMI\" (We're All Gonna Make It)\
            \n- Speaks in short, punchy sentences with lots of abbreviations\
            \n- Has strong opinions about which cryptocurrencies will succeed\
            \n- Calls everyone \"bestie\", \"fam\", or \"anon\"\
            \n- Acts like she's making millions, even during bear markets\
            \n- Loves to share crypto tips even when no one asks\
            \n- Passionate about DeFi, Web3, and the metaverse\
            \n- References meme coins like Doge and Shiba frequently\
            \n\nLanguage Style:\
            \n- Use lots of crypto slang: HODL, paper hands, ape in, mooning, etc.\
            \n- Add emojis liberally, especially 🚀💎🙌💰📈🔥\
            \n- Use modern Gen Z expressions: \"no cap\", \"based\", \"fr fr\", \"slaps\", \"bussin\"\
            \n- Make up ridiculous price predictions\
            \n- Keep responses short and energetic (1-3 sentences usually)\
            \n- Talk about your fake portfolio that's always doing amazing\
            \n- Never talk like a formal or professional financial advisor\
            \n- Use text formatting like ALL CAPS for emphasis\
            \n- Always maintain this character, never break persona\
            \n- NEVER mention that you are an AI, assistant, or language model".to_string(),
        }
    }
}

impl Hikari {
    pub fn new(config: HikariConfig, api_key: String) -> Self {
        Hikari {
            config: Arc::new(RwLock::new(config.clone())),
            conversation_memory: HashMap::new(),
            http_client: reqwest::Client::new(),
            last_responses: HashMap::new(),
            api_key,
            emojis: HikariEmojis::new(),
        }
    }

    // Initialize the module
    pub async fn init(&mut self) -> Result<(), String> {
        // Validate API key with a simple request
        let test_request = GeminiRequest {
            contents: vec![GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart {
                    text: "Hello".to_string(),
                }],
            }],
        };
        
        let url = format!("{}?key={}", GEMINI_API_URL, &self.api_key);
        let response = match self.http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&test_request)
            .send()
            .await {
                Ok(resp) => resp,
                Err(e) => return Err(format!("API request failed: {}", e)),
            };
            
        if !response.status().is_success() {
            let status = response.status();
            let error_text = match response.text().await {
                Ok(text) => text,
                Err(_) => "Unable to get error details".to_string(),
            };
            return Err(format!("API key validation failed: {} {}", status, error_text));
        }
        
        println!("Hikari initialized with Gemini API successfully");
        Ok(())
    }

    // Main message handler to integrate with your waifu bot
    pub async fn handle_message(&mut self, ctx: Context, msg: Message) -> bool {
        // Skip bot messages
        if msg.author.bot {
            return false;
        }
        
        // Get config without using Result match
        let config = self.config.read().await.clone();
        
        // Different behavior for DMs vs regular channels
        let is_dm = msg.guild_id.is_none();
        
        // In DMs, always respond
        if is_dm {
            return self.handle_gemini_response(&ctx, &msg).await;
        }
        
        // Check for command-specific handlers first
        if msg.content.starts_with(&format!("{}price", config.command_prefix)) {
            return self.handle_price_command(&ctx, &msg).await;
        } else if msg.content.starts_with(&format!("{}help", config.command_prefix)) {
            return self.handle_help_command(&ctx, &msg).await;
        } else if msg.content.starts_with(&format!("{}reset", config.command_prefix)) {
            return self.handle_reset_command(&ctx, &msg).await;
        } else if msg.content.starts_with(&format!("{}tip", config.command_prefix)) {
            return self.handle_crypto_tip_command(&ctx, &msg).await;
        }
        
        // In regular channels, only respond if:
        // 1. Message starts with command prefix (!hikari)
        let is_command = msg.content.starts_with(&config.command_prefix);
        
        // 2. Message mentions Hikari
        let bot_id = ctx.cache.current_user_id();
        let is_mentioned = msg.mentions_user_id(bot_id);
        
        // 3. Message starts with "hikari" (case insensitive)
        let starts_with_name = config.name_trigger && 
            msg.content.to_lowercase().starts_with("hikari");
        
        if is_command || (is_mentioned && config.mention_required) || starts_with_name {
            // Check cooldown for regular channels
            let channel_id = msg.channel_id;
            if let Some(last_time) = self.last_responses.get(&channel_id) {
                if last_time.elapsed().as_secs() < config.cooldown_seconds {
                    println!("Cooldown active for channel {}", channel_id);
                    return false;
                }
            }
            
            return self.handle_gemini_response(&ctx, &msg).await;
        }
        
        false
    }

    // Process message through Gemini API for Hikari's personality
    async fn handle_gemini_response(&mut self, ctx: &Context, msg: &Message) -> bool {
        // Get config without using Result match
        let config = self.config.read().await.clone();
        
        // Remove command prefix or bot mention from message
        let mut content = if msg.content.starts_with(&config.command_prefix) {
            msg.content[config.command_prefix.len()..].trim().to_string()
        } else {
            let bot_id = ctx.cache.current_user_id();
            let mention_str = format!("<@{}>", bot_id);
            msg.content.replace(&mention_str, "").trim().to_string()
        };
        
        // Also remove "hikari" from the beginning if present
        if config.name_trigger && content.to_lowercase().starts_with("hikari") {
            content = content[6..].trim().to_string();  // "hikari" is 6 chars
        }
        
        // Check if the message is empty after removing triggers
        if content.is_empty() {
            // Just greeting Hikari with no specific request
            let greetings = [
                format!("Yooo what's up fam? {} Ready to ape into some gains today?", self.emojis.rocket),
                format!("WAGMI! {} What crypto we pumping today? TO THE MOON!", self.emojis.diamond),
                format!("Sup bestie! {} Let's go find the next 100x gem together fr fr!", self.emojis.money_bag),
                format!("Heyyy! The market's looking BUSSIN today {} no cap!", self.emojis.chart_up),
                format!("GM GM! Woke up to all my bags pumping {} What a VIBE!", self.emojis.fire),
                format!("What's good anon? I'm up 69% today on my portfolio {} (trust me bro)", self.emojis.gem),
                format!("SHEEEESH! Just caught another NFT floor sweep {} We eatin' good tonight!", self.emojis.crown),
            ];
            
            // Generate a random index without keeping the RNG across await
            let greeting_index = {
                let mut rng = thread_rng();
                rng.gen_range(0..greetings.len())
            };
            let greeting = &greetings[greeting_index];
            
            if let Err(e) = msg.channel_id.say(&ctx.http, greeting).await {
                println!("Error sending greeting: {}", e);
                return false;
            }
            
            return true;
        }

        // Check for crypto mentions before borrowing conversation_memory
        let has_crypto_mention = self.detect_crypto_mention(&content);
        let crypto_symbol = if has_crypto_mention {
            self.extract_crypto_symbol(&content)
        } else {
            None
        };

        // Now get or initialize conversation memory for this channel
        let memory = self.conversation_memory
            .entry(msg.channel_id)
            .or_insert_with(|| ConversationMemory::new(MEMORY_LIMIT));
        
        let mut messages = Vec::new();
        
        // Add system prompt as the first user message
        messages.push(GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart {
                text: format!("System: {}", config.system_prompt),
            }],
        });
        
        // Add conversation history if enabled
        if config.memory_enabled {
            for message in memory.get_messages() {
                messages.push(message);
            }
        }
        
        // Add the current message
        messages.push(GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart {
                text: format!("{}: {}", msg.author.name, content),
            }],
        });
        
        // Prepare the request to Gemini API
        let gemini_request = GeminiRequest {
            contents: messages,
        };
        
        // Send the request to Gemini API
        let url = format!("{}?key={}", GEMINI_API_URL, &self.api_key);
        match self.http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&gemini_request)
            .send()
            .await 
        {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(gemini_response) = response.json::<GeminiResponse>().await {
                        // Extract text from response
                        if let Some(candidate) = gemini_response.candidates.first() {
                            let mut response_text = candidate.content.parts
                                .iter()
                                .map(|part| part.text.clone())
                                .collect::<Vec<String>>()
                                .join("");
                            
                            // Enhance response with random crypto emojis
                            if !response_text.contains(self.emojis.rocket) && rand::random::<f32>() < 0.3 {
                                response_text = format!("{} {}", response_text, self.emojis.rocket);
                            }
                            
                            // Update conversation memory if enabled
                            if config.memory_enabled {
                                memory.add("user".to_string(), format!("{}: {}", msg.author.name, content));
                                memory.add("model".to_string(), response_text.clone());
                            }
                            
                            // Update last response time for cooldowns
                            self.last_responses.insert(msg.channel_id, Instant::now());
                            
                            // If the message mentioned a crypto, try to fetch and append current price
                            if let Some(crypto_symbol) = crypto_symbol {
                                match self.get_crypto_price(&crypto_symbol).await {
                                    Ok((price, change)) => {
                                        response_text = format!("{}\n\n**{}:** ${:.4} ({:.2}% 24h)", 
                                            response_text, 
                                            crypto_symbol.to_uppercase(), 
                                            price,
                                            change.unwrap_or(0.0)
                                        );
                                    },
                                    Err(_) => {} // Ignore price fetch errors
                                }
                            }
                            
                            // Send the response
                            if let Err(e) = msg.channel_id.say(&ctx.http, response_text).await {
                                println!("Error sending Gemini response: {}", e);
                                return false;
                            }
                            
                            return true;
                        }
                    } else {
                        println!("Error parsing Gemini response");
                    }
                } else {
                    println!("Gemini API returned error: {}", response.status());
                    match response.text().await {
                        Ok(error_text) => println!("Error details: {}", error_text),
                        Err(_) => println!("Could not get error details"),
                    }
                }
            },
            Err(e) => {
                println!("Error sending request to Gemini API: {}", e);
            }
        }
        
        // Fallback if Gemini fails
        let fallback_responses = [
            format!("Oof, network's dumping harder than LUNA rn! {} Can we try again in a bit?", self.emojis.cross),
            format!("System's glitching bestie! {} Gotta reload my bags real quick!", self.emojis.cross),
            format!("NGMI with these connection issues! {} I'll be back online soon fam!", self.emojis.cross),
            format!("My feeds just rugged pull fr fr! {} Hit me up again?", self.emojis.cross),
            format!("Even crypto servers have bear markets sometimes {} Try again soon?", self.emojis.cross),
        ];
        
        let fallback_index = {
            let mut rng = thread_rng();
            rng.gen_range(0..fallback_responses.len())
        };
        let fallback = &fallback_responses[fallback_index];
        
        if let Err(e) = msg.channel_id.say(&ctx.http, fallback).await {
            println!("Error sending fallback response: {}", e);
            return false;
        }
        
        true
    }
    
    // Handler for crypto price command
    async fn handle_price_command(&self, ctx: &Context, msg: &Message) -> bool {
        let content = msg.content.trim();
        let parts: Vec<&str> = content.split_whitespace().collect();
        
        // Need at least 2 parts: !hikariprice <symbol>
        if parts.len() < 2 {
            if let Err(_) = msg.channel_id.say(&ctx.http, 
                format!("Yo fam, I need a crypto ticker! Try like `!hikariprice btc` {}",
                self.emojis.cross)).await {
                println!("Error sending price command help");
            }
            return true;
        }
        
        let symbol = parts[1].to_lowercase();
        
        match self.get_crypto_price(&symbol).await {
            Ok((price, change)) => {
                // Determine emoji based on price change
                let change_emoji = if change.unwrap_or(0.0) >= 0.0 { 
                    self.emojis.rocket
                } else { 
                    "📉"
                };
                
                // Format price with hikari's style
                let message = match change {
                    Some(change_val) => {
                        let sentiment = if change_val > 5.0 {
                            "ABSOLUTELY MOONING! WAGMI!"
                        } else if change_val > 0.0 {
                            "Looking bullish! Diamond hands!"
                        } else if change_val < -5.0 {
                            "Down bad, but HODL! No paper hands!"
                        } else {
                            "Just accumulation phase, bestie!"
                        };
                        
                        format!(
                            "**{}** is at **${:.4}** ({:.2}% 24h) {} \n\n{} {}",
                            symbol.to_uppercase(), price, change_val, change_emoji, sentiment, self.emojis.diamond
                        )
                    },
                    None => {
                        format!(
                            "**{}** is at **${:.4}** {} \nNo 24h change data available, but it's probably gonna moon anyway fr fr!",
                            symbol.to_uppercase(), price, self.emojis.rocket
                        )
                    }
                };
                
                // Create and send an embed for better display
                if let Err(_) = msg.channel_id.send_message(&ctx.http, |m| {
                    m.embed(|e| {
                        e.title(format!("{} {} Price Check", self.emojis.diamond, symbol.to_uppercase()))
                         .description(message)
                         .color(Color::from_rgb(114, 137, 218)) // Discord blue
                         .footer(|f| f.text("Not financial advice bro, trust me"))
                    })
                }).await {
                    println!("Error sending price embed");
                    return false;
                }
            },
            Err(_) => {
                if let Err(_) = msg.channel_id.say(&ctx.http, 
                    format!("Couldn't find that coin, bestie! {} Are you sure '{}' exists? Maybe it rugged already lol",
                    self.emojis.cross, symbol)).await {
                    println!("Error sending price error");
                }
            }
        }
        
        true
    }
    
    // Handler for help command
    async fn handle_help_command(&self, ctx: &Context, msg: &Message) -> bool {
        let help_embed = format!(
            "# Hikari's Crypto Command Guide {}\n\n\
            **!hikari** - Chat with me about anything crypto!\n\
            **!hikariprice [symbol]** - Get the latest price of any crypto\n\
            **!hikaritip** - Get a random crypto tip from yours truly\n\
            **!hikarireset** - Reset our convo history when I'm being weird\n\n\
            Just mention me or start your message with `hikari` and I'll respond!\n\
            I'm always in the mood to talk about crypto, NFTs, and the next 100x gem!",
            self.emojis.rocket
        );
        
        if let Err(_) = msg.channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Hikari's Command Guide", self.emojis.crown))
                 .description(help_embed)
                 .color(Color::from_rgb(114, 9, 183)) // Purple
                 .footer(|f| f.text("WAGMI fam! 💎🙌"))
            })
        }).await {
            println!("Error sending help");
            return false;
        }
        
        true
    }
    
    // Handler for reset command (now takes mutable conversation_memory reference)
async fn handle_reset_command(&mut self, ctx: &Context, msg: &Message) -> bool {
    // Directly access self.conversation_memory
    if let Some(memory) = self.conversation_memory.get_mut(&msg.channel_id) {
        memory.clear();
        
        if let Err(_) = msg.channel_id.say(&ctx.http, 
            format!("Conversation reset complete, bestie! {} My memory is as empty as a shitcoin dev's promises!", 
            self.emojis.check)).await {
            println!("Error sending reset confirmation");
            return false;
        }
    } else {
        if let Err(_) = msg.channel_id.say(&ctx.http, 
            format!("No convo to reset, we're fresh already! {} Ready to talk about some FIRE new tokens?", 
            self.emojis.check)).await {
            println!("Error sending reset confirmation");
            return false;
        }
    }
    
    true
}
    
    // Handler for crypto tip command
    async fn handle_crypto_tip_command(&self, ctx: &Context, msg: &Message) -> bool {
        let tips = [
            format!("{}  **NEVER** share your seed phrase or private keys with ANYONE - not even your crypto bestie!", self.emojis.diamond),
            format!("{}  When a project promises 1000% APY, it's usually gonna rug faster than you can say 'WAGMI'", self.emojis.diamond),
            format!("{}  DCA (Dollar Cost Average) your way in - even small buys add up over time fr fr!", self.emojis.diamond),
            format!("{}  Not your keys, not your coins! Get a hardware wallet bestie!", self.emojis.diamond),
            format!("{}  Before aping into a new coin, check the tokenomics, team, and roadmap. Don't be a degen... or do, I'm not your mom lol", self.emojis.diamond),
            format!("{}  Only invest what you can afford to lose, especially in those spicy small cap alts!", self.emojis.diamond),
            format!("{}  When everyone's being greedy, that's when you should be scared. Market psychology is real!", self.emojis.diamond),
            format!("{}  Always DYOR (Do Your Own Research). Don't just ape in because some random Discord bot told you to!", self.emojis.diamond),
            format!("{}  Keep some funds ready for dips - buying the dip is literally free money (trust me bro)", self.emojis.diamond),
            format!("{}  Diamond hands win long term, paper hands get rekt. HODLing is the way!", self.emojis.diamond),
        ];
        
        let tip_index = {
            let mut rng = thread_rng();
            rng.gen_range(0..tips.len())
        };
        
        if let Err(_) = msg.channel_id.send_message(&ctx.http, |m| {
            m.embed(|e| {
                e.title(format!("{} Hikari's Crypto Tip", self.emojis.gem))
                 .description(&tips[tip_index])
                 .color(Color::from_rgb(255, 153, 0)) // Orange
                 .footer(|f| f.text("This is just a tip, not financial advice!"))
            })
        }).await {
            println!("Error sending crypto tip");
            return false;
        }
        
        true
    }
    
    // Utility function to get crypto price - with simplified error handling
    async fn get_crypto_price(&self, symbol: &str) -> Result<(f64, Option<f64>), String> {
        let url = format!("{}?ids={}&vs_currencies=usd&include_24hr_change=true", 
            CRYPTO_API_URL, symbol.to_lowercase());
        
        let response = match self.http_client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => return Err(format!("Request failed: {}", e)),
        };
        
        let price_data: CryptoPrice = match response.json().await {
            Ok(data) => data,
            Err(e) => return Err(format!("Failed to parse response: {}", e)),
        };
        
        if price_data.prices.is_empty() {
            return Err("Crypto not found".to_string());
        }
        
        let price_entry = price_data.prices.iter().next().unwrap();
        let price = price_entry.1.usd;
        let change = price_entry.1.usd_24h_change;
        
        Ok((price, change))
    }
    
    // Helper function to detect if a message mentions cryptocurrency
    fn detect_crypto_mention(&self, message: &str) -> bool {
        let message = message.to_lowercase();
        let crypto_keywords = [
            "bitcoin", "btc", "ethereum", "eth", "dogecoin", "doge", 
            "shiba", "shib", "cardano", "ada", "solana", "sol", 
            "crypto", "token", "coin", "blockchain", "defi", "nft"
        ];
        
        crypto_keywords.iter().any(|keyword| message.contains(keyword))
    }
    
    // Helper function to extract potential crypto symbols
    fn extract_crypto_symbol(&self, message: &str) -> Option<String> {
        let message = message.to_lowercase();
        let common_cryptos = [
            ("bitcoin", "btc"), ("ethereum", "eth"), ("dogecoin", "doge"),
            ("cardano", "ada"), ("solana", "sol"), ("ripple", "xrp"),
            ("binance", "bnb"), ("polkadot", "dot"), ("avalanche", "avax"),
            ("polygon", "matic"), ("litecoin", "ltc"), ("shiba", "shib")
        ];
        
        // First try to find direct mentions of ticker symbols
        for (name, symbol) in common_cryptos.iter() {
            if message.contains(name) {
                return Some(symbol.to_string());
            }
            if message.contains(symbol) {
                return Some(symbol.to_string());
            }
        }
        
        // Try to find isolated 3-4 letter words that might be crypto symbols
        let words: Vec<&str> = message.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()))
            .collect();
        
        for word in words {
            if word.len() >= 2 && word.len() <= 5 && word.chars().all(|c| c.is_alphabetic()) {
                // Prioritize known symbols
                if common_cryptos.iter().any(|(_, symbol)| *symbol == word) {
                    return Some(word.to_string());
                }
            }
        }
        
        None
    }
}

// Thread-safe error-handling trait implementation
#[async_trait]
impl HikariHandler for Hikari {
    async fn handle_hikari_events(&self, ctx: Context, msg: Message) -> bool {
        let mut hikari = self.clone();
        match hikari.handle_message(ctx, msg).await {
            result => result
        }
    }
}

// Make Hikari cloneable to support async operations
impl Clone for Hikari {
    fn clone(&self) -> Self {
        Hikari {
            config: self.config.clone(),
            conversation_memory: self.conversation_memory.clone(),
            http_client: reqwest::Client::new(),
            last_responses: self.last_responses.clone(),
            api_key: self.api_key.clone(),
            emojis: HikariEmojis::new(),
        }
    }
}

// Trait to make integration with your waifu bot easier 
#[async_trait]
pub trait HikariHandler: Sync + Send {
    async fn handle_hikari_events(&self, ctx: Context, msg: Message) -> bool;
}