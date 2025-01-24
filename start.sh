#!/bin/bash

# Check if .env file exists
if [ ! -f .env ]; then
    echo "Error: .env file not found!"
    echo "Please create a .env file with your Discord token:"
    echo "DISCORD_TOKEN=your_token_here"
    exit 1
fi

# Load environment variables
export $(cat .env | xargs)

# Build and run the bot
echo "Building Waifu Bot..."
cargo build --release

echo "Starting Waifu Bot..."
./target/release/waifu-bot
