use crate::github::GithubClient;
use crate::state::{PrData, StateManager};
use log::error;
use regex::Regex;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{LinkPreviewOptions, MessageReactionUpdated, ParseMode, ReactionType};

pub async fn handle_reaction(
    bot: Bot,
    update: MessageReactionUpdated,
    state: Arc<StateManager>,
) -> ResponseResult<()> {
    let message_id = update.message_id;
    let chat_id = update.chat.id;

    let user = if let Some(u) = update.user {
        u
    } else {
        return Ok(());
    };

    let username = user.username.clone().unwrap_or(user.first_name.clone());

    // Check if we track this message
    let mut data = match state.get_pr_data(message_id.0.to_string(), chat_id.0).await {
        Ok(Some(d)) => d,
        Ok(None) => return Ok(()),
        Err(e) => {
            error!("Error fetching PR data: {}", e);
            return Ok(());
        }
    };

    let old_emojis: Vec<String> = update
        .old_reaction
        .iter()
        .filter_map(|r| match r {
            ReactionType::Emoji { emoji } => Some(emoji.clone()),
            _ => None,
        })
        .collect();

    let new_emojis: Vec<String> = update
        .new_reaction
        .iter()
        .filter_map(|r| match r {
            ReactionType::Emoji { emoji } => Some(emoji.clone()),
            _ => None,
        })
        .collect();

    // specific emojis (Base characters)
    let heart = "\u{2764}"; // ❤
    let thumbs_up = "\u{1f44d}"; // 👍
    let ok_hand = "\u{1f44c}"; // 👌
    let cry = "\u{1f62d}"; // 😭
    let hundred = "\u{1f4af}"; // 💯
    let pray = "\u{1f64f}"; // 🙏
    let cooking = "\u{1f373}"; // 🍳

    let has_reaction =
        |list: &[String], base: &str| -> bool { list.iter().any(|e| e.starts_with(base)) };

    // Helper to update lists
    // Iterate over old emojis to remove them
    for emoji in &old_emojis {
        if !new_emojis.contains(emoji) {
            if emoji.starts_with(heart) {
                data.reviewers.retain(|u| u != &username);
            } else if emoji.starts_with(thumbs_up) {
                data.approvals.retain(|u| u != &username);
            } else if emoji.starts_with(cry) {
                // cry removes from reviewers when ADDED, so removing cry does nothing special?
                // Or maybe restores? For now, nothing.
            } else if emoji.starts_with(hundred) {
                // Managed by is_merged logic below?
                // actually we should handle it here or below.
                // Current logic handles toggles below.
            } else if emoji.starts_with(cooking) {
                // Managed below
            } else if emoji.starts_with(pray) {
                // Managed below
            } else {
                // It was a comment
                data.comments.retain(|u| u != &username);
            }
        }
    }

    // Iterate over new emojis to add them
    for emoji in &new_emojis {
        if !old_emojis.contains(emoji) {
            if emoji.starts_with(heart) {
                if !data.reviewers.contains(&username) {
                    data.reviewers.push(username.clone());
                }
            } else if emoji.starts_with(thumbs_up) {
                if !data.approvals.contains(&username) {
                    data.approvals.push(username.clone());
                }
            } else if emoji.starts_with(cry) {
                data.reviewers.retain(|u| u != &username);
            } else if emoji.starts_with(hundred) {
                data.is_merged = true;
            } else if emoji.starts_with(cooking) {
                data.is_draft = true;
            } else if emoji.starts_with(pray) {
                data.re_review_requested = true;
                // remove comments when re-review is requested via emoji
                data.comments.clear();
            } else {
                // It is a comment (including ok_hand)
                if !data.comments.contains(&username) {
                    data.comments.push(username.clone());
                }

                // If it is ok_hand, they reviewed it, so remove from reviewers list if they are there
                // (Assuming "reviewer" means "committed to review" and "comment/ok_hand" means "did review")
                if emoji.starts_with(ok_hand) {
                    data.reviewers.retain(|u| u != &username);
                }
            }
        }
    }

    // Handle toggles off for single-state booleans (merged, draft, re-review)
    // If specific emoji was removed
    if has_reaction(&old_emojis, hundred) && !has_reaction(&new_emojis, hundred) {
        data.is_merged = false;
    }
    if has_reaction(&old_emojis, cooking) && !has_reaction(&new_emojis, cooking) {
        data.is_draft = false;
    }
    if has_reaction(&old_emojis, pray) && !has_reaction(&new_emojis, pray) {
        data.re_review_requested = false;
    }

    // Save and Update Message
    if let Err(e) = state
        .update_pr_data(message_id.0.to_string(), data.clone())
        .await
    {
        error!("Failed to save state: {}", e);
    }

    let new_text = generate_message_text(&data);

    bot.edit_message_text(chat_id, message_id, new_text)
        .parse_mode(ParseMode::Html)
        .link_preview_options(LinkPreviewOptions {
            is_disabled: true,
            url: None,
            prefer_small_media: false,
            prefer_large_media: false,
            show_above_text: false,
        })
        .await?;

    Ok(())
}

pub async fn handle_message(
    bot: Bot,
    msg: Message,
    state: Arc<StateManager>,
    github: Arc<GithubClient>,
) -> ResponseResult<()> {
    let text = msg.text().unwrap_or("").to_string();

    // Check for /upgrade command
    if text.starts_with("/upgrade") {
        if let Some(reply) = msg.reply_to_message() {
            // "remove and upgrade to your message replied to message"
            // Case 1: Reply to a normal message with a link
            // Case 2: Reply to a bot message to refresh it? (Less likely intended meaning)
            // Most likely: User posted a link, bot didn't see it or it was before bot, user wants bot to "take over" that link.
            // Action: Parse link from replied message, delete replied message, post new bot message with tracking.

            let reply_text = reply.text().unwrap_or("");
            if let Some((owner, repo, pr_number)) = extract_pr_info(reply_text) {
                // Fetch PR info
                match github.get_pr_details(&owner, &repo, pr_number).await {
                    Ok(pr) => {
                        // Delete user message
                        bot.delete_message(msg.chat.id, reply.id).await?;
                        // Delete command message
                        bot.delete_message(msg.chat.id, msg.id).await?;

                        // Send new tracked message
                        let pr_data = PrData {
                            pr_url: pr.html_url.map(|u| u.to_string()).unwrap_or_default(),
                            title: pr.title.unwrap_or_default(),
                            author: pr.user.map(|u| u.login).unwrap_or("unknown".to_string()),
                            repo: format!("{}/{}", owner, repo),
                            pr_number,
                            reviewers: vec![],
                            approvals: vec![],
                            comments: vec![],
                            is_merged: pr.merged_at.is_some(),
                            is_draft: pr.draft.unwrap_or(false),
                            re_review_requested: false,
                            chat_id: msg.chat.id.0,
                        };

                        let text = generate_message_text(&pr_data);
                        let sent_msg = bot
                            .send_message(msg.chat.id, text)
                            .parse_mode(ParseMode::Html)
                            .link_preview_options(LinkPreviewOptions {
                                is_disabled: true,
                                url: None,
                                prefer_small_media: false,
                                prefer_large_media: false,
                                show_above_text: false,
                            })
                            .await?;

                        state
                            .add_message(sent_msg.id.0.to_string(), pr_data)
                            .await
                            .ok();

                        // Add repo to tracking if new
                        state.add_repository(&owner, &repo).await.ok();
                    }
                    Err(e) => {
                        error!("Failed to fetch PR: {}", e);
                        bot.send_message(msg.chat.id, "Failed to fetch PR details.")
                            .await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Help command
    if text.starts_with("/help") || text.starts_with("/start") {
        let help_text = r#"
<b>🤖 PR Monitor Bot Help</b>

I monitor GitHub PRs and track review status via emojis or commands.

<b>Commands (reply to tracked message):</b>
/review - Mark as reviewing (❤️)
/approve - Approve PR (👍)
/comment - Add comment status (👌)
/giveup - Unassign self (😭)
/merge - Mark as merged (💯)
/draft - Mark as draft (🍳)
/addressed or /rereview - Request re-review (🙏)

<b>General Commands:</b>
/upgrade (reply to link) - Replace link with tracked message
/help - Show this message
"#;
        bot.send_message(msg.chat.id, help_text)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    // Interactive commands (reply based)
    if let Some(reply_to) = msg.reply_to_message() {
        let parent_id = reply_to.id;

        // Check if it's a tracked message
        if let Ok(Some(mut data)) = state
            .get_pr_data(parent_id.0.to_string(), msg.chat.id.0)
            .await
        {
            let mut changed = false;
            let username = msg
                .from
                .as_ref()
                .map(|u| u.username.clone().unwrap_or(u.first_name.clone()))
                .unwrap_or("unknown".to_string());

            if text.starts_with("/addressed") || text.starts_with("/rereview") {
                data.re_review_requested = true;
                // remove comments when re-review is requested
                data.comments.clear();
                changed = true;
            } else if text.starts_with("/review") {
                if !data.reviewers.contains(&username) {
                    data.reviewers.push(username);
                    changed = true;
                }
            } else if text.starts_with("/approve") {
                if !data.approvals.contains(&username) {
                    data.approvals.push(username);
                    changed = true;
                }
            } else if text.starts_with("/comment") {
                if !data.comments.contains(&username) {
                    data.comments.push(username);
                    changed = true;
                }
            } else if text.starts_with("/giveup") {
                data.reviewers.retain(|u| u != &username);
                changed = true;
            } else if text.starts_with("/merge") {
                data.is_merged = true;
                changed = true;
            } else if text.starts_with("/draft") {
                data.is_draft = !data.is_draft; // Toggle draft
                changed = true;
            }

            if changed {
                if let Err(e) = state
                    .update_pr_data(parent_id.0.to_string(), data.clone())
                    .await
                {
                    error!("Failed to save state: {}", e);
                }

                let new_text = generate_message_text(&data);
                bot.edit_message_text(msg.chat.id, parent_id, new_text)
                    .parse_mode(ParseMode::Html)
                    .link_preview_options(LinkPreviewOptions {
                        is_disabled: true,
                        url: None,
                        prefer_small_media: false,
                        prefer_large_media: false,
                        show_above_text: false,
                    })
                    .await?;

                // Delete the command message
                bot.delete_message(msg.chat.id, msg.id).await.ok();
                return Ok(());
            }
        }
    }

    // Check for /addressed command (Legacy specific block removed as merged above)

    // Check if reply to a tracked message (Re-review logic)
    if let Some(reply_to) = msg.reply_to_message() {
        let parent_id = reply_to.id;
        if let Ok(Some(mut data)) = state
            .get_pr_data(parent_id.0.to_string(), msg.chat.id.0)
            .await
        {
            if text.contains("http") || text.contains("github.com") {
                data.re_review_requested = true;
                // remove comments when re-review is requested
                data.comments.clear();
                if let Err(e) = state
                    .update_pr_data(parent_id.0.to_string(), data.clone())
                    .await
                {
                    error!("Failed to save state: {}", e);
                }
                let new_text = generate_message_text(&data);
                bot.edit_message_text(msg.chat.id, parent_id, new_text)
                    .parse_mode(ParseMode::Html)
                    .link_preview_options(LinkPreviewOptions {
                        is_disabled: true,
                        url: None,
                        prefer_small_media: false,
                        prefer_large_media: false,
                        show_above_text: false,
                    })
                    .await?;
            }
        }
    }

    // "parse messages from other parties and if it is a link replace with your message"
    // Check if message contains a PR link
    if let Some((owner, repo, pr_number)) = extract_pr_info(&text) {
        // If message is from bot, ignore (should allow loop prevention)
        if let Some(user) = msg.from {
            if user.is_bot {
                // assume it's us or another bot, maybe we shouldn't replace it if it's us?
                // But `handle_message` usually doesn't trigger for own messages unless configured.
            } else {
                match github.get_pr_details(&owner, &repo, pr_number).await {
                    Ok(pr) => {
                        // Delete user message
                        bot.delete_message(msg.chat.id, msg.id).await?;

                        let pr_data = PrData {
                            pr_url: pr.html_url.map(|u| u.to_string()).unwrap_or_default(),
                            title: pr.title.unwrap_or_default(),
                            author: pr.user.map(|u| u.login).unwrap_or("unknown".to_string()),
                            repo: format!("{}/{}", owner, repo),
                            pr_number,
                            reviewers: vec![],
                            approvals: vec![],
                            comments: vec![],
                            is_merged: pr.merged_at.is_some(),
                            is_draft: pr.draft.unwrap_or(false),
                            re_review_requested: false,
                            chat_id: msg.chat.id.0,
                        };

                        let text = generate_message_text(&pr_data);
                        let sent_msg = bot
                            .send_message(msg.chat.id, text)
                            .parse_mode(ParseMode::Html)
                            .link_preview_options(LinkPreviewOptions {
                                is_disabled: true,
                                url: None,
                                prefer_small_media: false,
                                prefer_large_media: false,
                                show_above_text: false,
                            })
                            .await?;

                        state
                            .add_message(sent_msg.id.0.to_string(), pr_data)
                            .await
                            .ok();
                        state.add_repository(&owner, &repo).await.ok();
                    }
                    Err(e) => error!("Failed to fetch PR: {}", e),
                }
            }
        }
    }

    Ok(())
}

fn extract_pr_info(text: &str) -> Option<(String, String, u64)> {
    let re = Regex::new(r"github\.com/([^/]+)/([^/]+)/pull/(\d+)").unwrap();
    if let Some(captures) = re.captures(text) {
        let owner = captures.get(1)?.as_str().to_string();
        let repo = captures.get(2)?.as_str().to_string();
        let number = captures.get(3)?.as_str().parse::<u64>().ok()?;
        return Some((owner, repo, number));
    }
    None
}

fn generate_message_text(data: &PrData) -> String {
    let mut text = format!(
        "<b>PR:</b> <a href=\"{}\">{}</a>\n",
        data.pr_url, data.title
    );
    text.push_str(&format!("<b>Author:</b> {}\n", data.author));
    text.push_str(&format!("<b>Repo:</b> {}\n\n", data.repo));

    if data.is_merged {
        text.push_str("<b>Status:</b> 💯 MERGED\n\n");
    } else if data.is_draft {
        text.push_str("<b>Status:</b> 🍳 Draft/WIP\n\n");
    }

    if data.re_review_requested {
        text.push_str("🙏 <b>Re-review Requested!</b>\n\n");
    }

    if !data.reviewers.is_empty() {
        text.push_str(&format!(
            "❤️ <b>Reviewers:</b> {}\n",
            data.reviewers.join(", ")
        ));
    }
    if !data.approvals.is_empty() {
        text.push_str(&format!(
            "👍 <b>Approved:</b> {}\n",
            data.approvals.join(", ")
        ));
    }
    if !data.comments.is_empty() {
        text.push_str(&format!(
            "👌 <b>Comments:</b> {}\n",
            data.comments.join(", ")
        ));
    }

    text
}
