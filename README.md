# Telegram Bot for GitHub PR Monitoring

> ⚠️ **Warning**: This project is **vibe coded**. Use at your own risk.

A Telegram bot that monitors GitHub repositories for new Pull Requests, tracks their review status via emojis or commands, and automatically cleans up closed or merged PRs.

## Features

- **Automated Monitoring**: Checks for new PRs in configured repositories every minute.
- **Review Tracking**: Uses Telegram reactions or commands to track review status.
  - ❤️ / `/review` - Mark as "Reviewing"
  - 👍 / `/approve` - Mark as "Approved"
  - 👌 / `/comment` - Mark as "Commented"
  - 😭 / `/giveup` - Unassign self from review
  - 💯 / `/merge` - Mark as "Merged"
  - 🍳 / `/draft` - Toggle "Draft" status
  - 🙏 / `/addressed` / `/rereview` - Request re-review (clears previous comments)
- **Real-Time Synchronization**: The bot periodically syncs with GitHub to fetch the latest:
  - Review statuses (Approved, Changes Requested, Commented)
  - Draft status
  - This ensures the message always reflects the actual state on GitHub, treating GitHub as the source of truth.
- **Interactive Updates**: The bot updates the message text in real-time to reflect the current status (Reviewers, Approvals, Changes Requested, Comments).
- **Auto-Cleanup**: Automatically deletes messages for PRs that are closed or merged on GitHub.
- **Link Parsing**: If a user posts a GitHub PR link, the bot can replace it with a tracked message (via `/upgrade` or auto-detection).

## Setup

### Prerequisites

- Rust (latest stable)
- SQLite (if running locally without Docker)
- A Telegram Bot Token (from @BotFather)
- A GitHub Personal Access Token (PAT)

## Configuration

- `GITHUB_REPOS`: Comma-separated list of repositories to **fully monitor** (automatic new PR alerts + interactive tracking).
- `GITHUB_IGNORED_REPOS`: Comma-separated list of repositories to **ignore for automatic alerts**.
  - New PRs will **NOT** be auto-posted.
  - However, you can still manually track PRs from these repos by replying to a link with `/upgrade` or pasting the link if auto-link-detection is enabled.

### Environment Variables

Environment variables:
```env
TELEGRAM_BOT_TOKEN=your_telegram_bot_token
GITHUB_TOKEN=your_github_pat
TELEGRAM_CHAT_ID=target_chat_id
GITHUB_REPOS=owner/repo1,owner/repo2
GITHUB_IGNORED_REPOS=owner/repo3,owner/repo4
DATABASE_URL=sqlite:bot.db
RUST_LOG=info
```

### Running Locally

```bash
# Install dependencies
cargo build

# Run the bot
cargo run
```

### Running with Docker

```bash
# Build the image
docker build -t tg-bot .

# Run the container
docker run -d \
  --env-file .env \
  -v $(pwd)/data:/app/data \
  tg-bot
```

## Usage

1. **Add the bot** to your Telegram group (defined by `TELEGRAM_CHAT_ID`).
2. **New PRs** will automatically appear in the chat.
3. **React** to the messages to change their status:
   - Click ❤️ to add yourself as a reviewer.
   - Click 👍 to approve.
   - Click 👌 to indicate you've commented.
   - Click 🙏 to request a re-review (this clears the comment list).
   - Click 💯 to manually mark as merged (though the bot auto-checks this too).
4. **Commands**:
   - Reply to a bot message with `/addressed` to request a re-review.
   - Reply to a raw GitHub link with `/upgrade` to convert it into a tracked bot message.
   - Send `/help` to see the full list of commands.

## Development

The project is built with:
- **Teloxide**: For Telegram bot API interactions.
- **Octocrab**: For GitHub API interactions.
- **SQLx**: For SQLite database persistence.
- **Tokio**: For async runtime and task scheduling.
