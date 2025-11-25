# Deployment Instructions (Coolify)

This bot is designed to be easily hosted on [Coolify](https://coolify.io/).

## Prerequisites
- A Telegram Bot Token (from @BotFather)
- A Telegram Chat ID (where the bot will post)
- A GitHub Personal Access Token (for API access)

## Setup in Coolify

1. **Create a Service**: Select "Rust" or "Docker" deployment source. Point it to your repository.
2. **Build Pack**: Use "Docker Compose" or "Dockerfile". The provided `Dockerfile` is ready to use.
3. **Environment Variables**:
   Add the following environment variables in Coolify:

   - `TELEGRAM_BOT_TOKEN`: Your bot token.
   - `GITHUB_TOKEN`: Your GitHub token.
   - `TELEGRAM_CHAT_ID`: The chat ID (integer).
   - `GITHUB_REPOS`: (Optional) Comma separated list of `owner/repo` to seed initial tracking. Example: `near/near-core,near/bos-cli-rs`.
   - `RUST_LOG`: `info` (for logging).
   - `DATABASE_URL`: `sqlite:/app/data/bot.db`

4. **Persistent Storage**:
   Important: You must configure a persistent volume for the SQLite database so data persists across restarts/redeployments.
   
   - Mount a volume to `/app/data`.
   - In Coolify, under "Storage", add a volume mapped to `/app/data`.

5. **Deploy**: Click deploy!

## Features

- **PR Monitoring**: Automatically checks for new PRs in tracked repos.
- **Interactive Tracking**: Tracks review status via emoji reactions (❤️, 👍, 👌, 😭, 💯, 🙏, 🍳).
- **Auto-link Parsing**: If anyone posts a GitHub PR link, the bot replaces it with a tracked message.
- **Dynamic Tracking**: Automatically adds new repositories from posted links.
- **Upgrade Command**: Reply with `/upgrade` to a message containing a PR link to replace it with a tracked bot message.


