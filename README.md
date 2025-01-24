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
git clone https://github.com/yourusername/waifu-bot
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

-------------------

# .gitignore
target/
waifu_bot_data/
.env
*.log
Cargo.lock

-------------------

# Dockerfile
FROM rust:1.75-slim

WORKDIR /usr/src/waifu-bot
COPY . .

RUN cargo build --release

CMD ["./target/release/waifu-bot"]

-------------------

# start.sh
#!/bin/bash

if [ ! -f .env ]; then
    echo "Error: .env file not found!"
    echo "Please create a .env file with your Discord token"
    exit 1
fi

export $(cat .env | xargs)
cargo run --release

-------------------

# LICENSE
MIT License

Copyright (c) 2024 Your Name

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
