# Waifu Bot

Discord bot for waifu character selection and profile management.

## Features
- Character selection system with 13 waifus
- Profile tracking and management
- Monthly character rotation
- Team/clan system
- Persistent data storage

## Requirements
- Rust (latest stable)
- Discord Bot Token
- Git

## Installation

1. Clone the repository:
```bash
git clone https://github.com/OSXBasedAnon/waifu-bot
cd waifu-bot
```

2. Create `.env` file:
```bash
DISCORD_TOKEN=your_bot_token
APPLICATION_ID=1332375710995845352
PUBLIC_KEY=8aa7fa4c783f07a0bc35a3e1024b8a7f3fad56b7c98791e2b86302250f017b30
```

3. Build and run:
```bash
cargo build --release
./target/release/waifu-bot
```

## Usage

### Commands
- `!profile` - View your profile
- `!rankings` - View current waifu rankings

### Character Selection
React to the welcome message with:
- <a:maki:1327355612350382141> - Maki
- <a:kaguya:1327356393467940996> - Kaguya
- <a:marin:1327356110449016934> - Marin
(etc...)

## Deployment

### Local
```bash
./start.sh
```

### Server (Screen)
```bash
screen -S waifu-bot
./target/release/waifu-bot
# Ctrl+A+D to detach
```

### Docker
```bash
docker build -t waifu-bot .
docker run -d waifu-bot
```

## License
MIT
