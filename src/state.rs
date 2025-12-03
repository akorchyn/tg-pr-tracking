use crate::db::Db;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrData {
    pub pr_url: String,
    pub title: String,
    pub author: String,
    pub repo: String, // "owner/repo"
    pub pr_number: u64,
    pub reviewers: Vec<String>,
    pub approvals: Vec<String>,
    pub changes_requested: Vec<String>,
    pub comments: Vec<String>,
    pub is_merged: bool,
    pub is_draft: bool,
    pub re_review_requested: bool,
    pub chat_id: i64,
}

#[derive(Clone)]
pub struct StateManager {
    db: Db,
}

impl StateManager {
    pub const fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn add_message(&self, message_id: String, data: PrData) -> Result<()> {
        let (owner, name) = {
            let parts: Vec<&str> = data.repo.split('/').collect();
            (parts[0].to_string(), parts[1].to_string())
        };

        let msg = crate::db::PrMessage {
            message_id: message_id.clone(),
            chat_id: data.chat_id,
            pr_url: data.pr_url,
            title: data.title,
            author: data.author,
            repo_owner: owner,
            repo_name: name,
            pr_number: data.pr_number as i64,
            is_merged: data.is_merged,
            is_draft: data.is_draft,
            re_review_requested: data.re_review_requested,
        };

        self.db.save_pr_message(&msg).await?;
        self.db
            .update_reactions(
                &message_id,
                data.chat_id,
                &data.reviewers,
                &data.approvals,
                &data.changes_requested,
                &data.comments,
            )
            .await?;
        
        // Mark seen
        let key = format!("{}#{}", data.repo, data.pr_number);
        self.db.mark_pr_seen(&key).await?;
        
        Ok(())
    }

    pub async fn get_pr_data(&self, message_id: String, chat_id: i64) -> Result<Option<PrData>> {
        let msg = self.db.get_pr_message(&message_id, chat_id).await?;
        if let Some(m) = msg {
            let (reviewers, approvals, changes_requested, comments) =
                self.db.get_reactions(&message_id, chat_id).await?;
            Ok(Some(PrData {
                pr_url: m.pr_url,
                title: m.title,
                author: m.author,
                repo: format!("{}/{}", m.repo_owner, m.repo_name),
                pr_number: m.pr_number as u64,
                reviewers,
                approvals,
                changes_requested,
                comments,
                is_merged: m.is_merged,
                is_draft: m.is_draft,
                re_review_requested: m.re_review_requested,
                chat_id: m.chat_id,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update_pr_data(&self, message_id: String, data: PrData) -> Result<()> {
        self.add_message(message_id, data).await
    }

    pub async fn is_pr_seen(&self, repo: &str, pr_number: u64) -> Result<bool> {
        let key = format!("{}#{}", repo, pr_number);
        self.db.is_pr_seen(&key).await
    }
    
    pub async fn add_repository(&self, owner: &str, name: &str) -> Result<()> {
        self.db.add_repository(owner, name).await
    }
    
    pub async fn get_repositories(&self) -> Result<Vec<(String, String)>> {
        let repos = self.db.get_repositories().await?;
        Ok(repos.into_iter().map(|r| (r.owner, r.name)).collect())
    }

    pub async fn get_all_active_messages(&self) -> Result<Vec<crate::db::PrMessage>> {
        self.db.get_all_active_messages().await
    }

    pub async fn remove_message(&self, message_id: &str, chat_id: i64) -> Result<()> {
        self.db.remove_message(message_id, chat_id).await
    }
}
