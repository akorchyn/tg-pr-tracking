use chrono::Utc;
use log::{error, info};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{LinkPreviewOptions, MessageId, ParseMode, Recipient};
use tokio::time::{sleep, Duration};

mod config;
mod db;
mod github;
mod handlers;
mod state;

use config::Config;
use db::Db;
use github::GithubClient;
use state::StateManager;

#[tokio::main]
async fn main() {
    env_logger::init();
    info!("Starting bot...");

    let config = Config::from_env().expect("Failed to load configuration");
    let bot = Bot::new(config.telegram_bot_token.clone());
    let github =
        GithubClient::new(config.github_token.clone()).expect("Failed to create Github client");

    // Initialize DB
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:bot.db".to_string());
    let db = Db::new(&database_url)
        .await
        .expect("Failed to connect to database");
    let state = Arc::new(StateManager::new(db));

    // Seed repositories from config
    for (owner, repo) in &config.repositories {
        state.add_repository(owner, repo).await.ok();
    }

    let bot_clone = bot.clone();
    let config_clone = config.clone();
    let github_clone = github.clone();
    let state_clone = state.clone();

    // Spawn GitHub monitoring task
    tokio::spawn(async move {
        let mut last_check = Utc::now() - chrono::Duration::minutes(1);

        loop {
            info!("Checking for new PRs...");
            // Fetch latest list of repos from DB
            let repos = state_clone.get_repositories().await.unwrap_or_default();
            let ignored_repos = config_clone.ignored_repositories.clone();

            for (owner, repo) in repos {
                // Skip if this repo is in the ignored list
                if ignored_repos.iter().any(|(o, r)| o == &owner && r == &repo) {
                    continue;
                }

                match github_clone.get_new_prs(&owner, &repo, last_check).await {
                    Ok(prs) => {
                        for pr in prs {
                            // Check if already seen using DB
                            if state_clone
                                .is_pr_seen(&repo, pr.id.0)
                                .await
                                .unwrap_or(false)
                            {
                                continue;
                            }

                            let title = pr.title.clone().unwrap_or_default();
                            let author = pr
                                .user
                                .clone()
                                .map(|u| u.login)
                                .unwrap_or("unknown".to_string());
                            let pr_url = pr
                                .html_url
                                .clone()
                                .map(|u| u.to_string())
                                .unwrap_or_default();

                            let msg = format!(
                                "New PR included:\n\nTitle: {}\nAuthor: {}\nRepo: {}/{}\nLink: {}",
                                title, author, owner, repo, pr_url
                            );

                            // Send to configured chat ID (for monitored PRs)
                            match bot_clone
                                .send_message(Recipient::Id(ChatId(config_clone.chat_id)), msg)
                                .await
                            {
                                Ok(sent_msg) => {
                                    // Fetch initial reviews (if any, though usually none on creation)
                                    let mut approvals = vec![];
                                    let mut changes_requested = vec![];
                                    let mut comments = vec![];

                                    if let Ok(reviews) =
                                        github_clone.get_pr_reviews(&owner, &repo, pr.number).await
                                    {
                                        for review in reviews {
                                            if let Some(user) = review.user {
                                                let username = user.login;
                                                match review.state {
                                                    Some(octocrab::models::pulls::ReviewState::Approved) => {
                                                        if !approvals.contains(&username) { approvals.push(username); }
                                                    },
                                                    Some(octocrab::models::pulls::ReviewState::ChangesRequested) => {
                                                        if !changes_requested.contains(&username) { changes_requested.push(username); }
                                                    },
                                                    Some(octocrab::models::pulls::ReviewState::Commented) => {
                                                        if !comments.contains(&username) { comments.push(username); }
                                                    },
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }

                                    // We don't automatically track *messages* sent by this loop as "interactive" unless we want to.
                                    // But the user requirements say "If it sees a new PR included, it will send a message... The review statuses are tracked using reactions"
                                    // So YES, we must track this message in DB so reactions work.

                                    let pr_data = state::PrData {
                                        pr_url,
                                        title,
                                        author,
                                        repo: format!("{}/{}", owner, repo),
                                        pr_number: pr.number,
                                        reviewers: vec![],
                                        approvals,
                                        changes_requested,
                                        comments,
                                        is_merged: pr.merged_at.is_some(),
                                        is_draft: pr.draft.unwrap_or(false),
                                        re_review_requested: false,
                                        chat_id: config_clone.chat_id,
                                    };
                                    state_clone
                                        .add_message(sent_msg.id.0.to_string(), pr_data)
                                        .await
                                        .ok();
                                }
                                Err(e) => error!("Failed to send message: {}", e),
                            }
                        }
                    }
                    Err(e) => error!("Failed to fetch PRs for {}/{}: {}", owner, repo, e),
                }
            }

            // Cleanup closed/merged PRs
            if let Ok(active_msgs) = state_clone.get_all_active_messages().await {
                for msg in active_msgs {
                    match github_clone
                        .get_pr_details(&msg.repo_owner, &msg.repo_name, msg.pr_number as u64)
                        .await
                    {
                        Ok(pr) => {
                            let is_closed =
                                matches!(pr.state, Some(octocrab::models::IssueState::Closed));
                            let is_merged = pr.merged_at.is_some();

                            // Update Draft status if changed
                            let current_draft = pr.draft.unwrap_or(false);
                            let mut data_changed = false;
                            let current_data_opt = state_clone
                                .get_pr_data(msg.message_id.clone(), msg.chat_id)
                                .await
                                .unwrap_or(None);

                            if let Some(mut data) = current_data_opt.clone() {
                                // Check draft status
                                if msg.is_draft != current_draft {
                                    info!(
                                        "PR {}/{}#{} draft status changed to {}. Updating...",
                                        msg.repo_owner, msg.repo_name, msg.pr_number, current_draft
                                    );
                                    data.is_draft = current_draft;
                                    data_changed = true;
                                }

                                // Sync reviews from GitHub
                                // Fetch reviews and requested reviewers
                                let mut new_approvals = vec![];
                                let mut new_changes_requested = vec![];
                                let mut new_comments = vec![];
                                let mut new_reviewers = vec![]; // Requested reviewers

                                // 1. Get actual reviews
                                if let Ok(reviews) = github_clone
                                    .get_pr_reviews(
                                        &msg.repo_owner,
                                        &msg.repo_name,
                                        msg.pr_number as u64,
                                    )
                                    .await
                                {
                                    // We need to deduplicate by user, taking the LATEST review state
                                    // Reviews are returned chronologically? API docs say "The list of reviews returns in chronological order."
                                    // So we can iterate and overwrite.

                                    // Map username -> state
                                    use std::collections::HashMap;
                                    let mut user_state: HashMap<
                                        String,
                                        octocrab::models::pulls::ReviewState,
                                    > = HashMap::new();

                                    for review in reviews {
                                        if let Some(user) = review.user {
                                            if let Some(state) = review.state {
                                                user_state.insert(user.login, state);
                                            }
                                        }
                                    }

                                    for (user, state) in user_state {
                                        match state {
                                             octocrab::models::pulls::ReviewState::Approved => new_approvals.push(user),
                                             octocrab::models::pulls::ReviewState::ChangesRequested => new_changes_requested.push(user),
                                             octocrab::models::pulls::ReviewState::Commented => new_comments.push(user),
                                             _ => {} // Dismissed, Pending, etc.
                                         }
                                    }
                                }

                                // Sort for consistent comparison
                                new_approvals.sort();
                                new_changes_requested.sort();
                                new_comments.sort();
                                new_reviewers.sort();

                                // Compare with existing data (which should also be sorted if we want strict equality, but vector equality checks elements)
                                // Actually, PrData vectors might not be sorted. Let's sort them for comparison.
                                data.approvals.sort();
                                data.changes_requested.sort();
                                data.comments.sort();
                                data.reviewers.sort();

                                if data.approvals != new_approvals
                                    || data.changes_requested != new_changes_requested
                                    || data.comments != new_comments
                                    || data.reviewers != new_reviewers
                                {
                                    info!(
                                        "PR {}/{}#{} review status changed. Syncing...",
                                        msg.repo_owner, msg.repo_name, msg.pr_number
                                    );
                                    data.approvals = new_approvals;
                                    data.changes_requested = new_changes_requested;
                                    data.comments = new_comments;
                                    data.reviewers = new_reviewers;
                                    data_changed = true;
                                }

                                if data_changed {
                                    if let Err(e) = state_clone
                                        .update_pr_data(msg.message_id.clone(), data.clone())
                                        .await
                                    {
                                        error!("Failed to update PR data in DB: {}", e);
                                    } else {
                                        // Update chat message
                                        let new_text = handlers::generate_message_text(&data);
                                        if let Err(e) = bot_clone
                                            .edit_message_text(
                                                ChatId(msg.chat_id),
                                                MessageId(msg.message_id.parse().unwrap_or(0)),
                                                new_text,
                                            )
                                            .parse_mode(ParseMode::Html)
                                            .link_preview_options(LinkPreviewOptions {
                                                is_disabled: true,
                                                url: None,
                                                prefer_small_media: false,
                                                prefer_large_media: false,
                                                show_above_text: false,
                                            })
                                            .await
                                        {
                                            error!("Failed to update PR message in chat: {}", e);
                                        }
                                    }
                                }
                            }

                            if is_closed || is_merged {
                                info!(
                                    "PR {}/{}#{} is closed/merged. Removing...",
                                    msg.repo_owner, msg.repo_name, msg.pr_number
                                );
                                // Delete message from chat
                                if let Err(e) = bot_clone
                                    .delete_message(
                                        ChatId(msg.chat_id),
                                        MessageId(msg.message_id.parse().unwrap_or(0)),
                                    )
                                    .await
                                {
                                    error!("Failed to delete message: {}", e);
                                }
                                // Remove from DB tracking
                                if let Err(e) = state_clone
                                    .remove_message(&msg.message_id, msg.chat_id)
                                    .await
                                {
                                    error!("Failed to remove message from DB: {}", e);
                                }
                            }
                        }
                        Err(e) => error!(
                            "Failed to check status for {}/{}#{}: {}",
                            msg.repo_owner, msg.repo_name, msg.pr_number, e
                        ),
                    }
                }
            }

            last_check = Utc::now();
            sleep(Duration::from_secs(120)).await;
        }
    });

    // Run Teloxide dispatcher
    let handler = dptree::entry()
        .branch(Update::filter_message_reaction_updated().endpoint(handlers::handle_reaction))
        .branch(Update::filter_message().endpoint(handlers::handle_message));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state, Arc::new(github)])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
