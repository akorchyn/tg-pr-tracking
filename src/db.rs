use anyhow::Result;
use sqlx::{sqlite::SqlitePool, FromRow, Row};

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

#[derive(FromRow, Debug)]
pub struct TrackedRepo {
    pub id: i64,
    pub owner: String,
    pub name: String,
}

#[derive(FromRow, Debug)]
pub struct PrMessage {
    pub message_id: String, // Stored as string to match existing logic, though sqlite handles int
    pub chat_id: i64,
    pub pr_url: String,
    pub title: String,
    pub author: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub pr_number: i64,
    pub is_merged: bool,
    pub is_draft: bool,
    pub re_review_requested: bool,
}

impl Db {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url).await?;
        let db = Self { pool };
        db.init().await?;
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                UNIQUE(owner, name)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS messages (
                message_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                pr_url TEXT NOT NULL,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                repo_owner TEXT NOT NULL,
                repo_name TEXT NOT NULL,
                pr_number INTEGER NOT NULL,
                is_merged BOOLEAN DEFAULT 0,
                is_draft BOOLEAN DEFAULT 0,
                re_review_requested BOOLEAN DEFAULT 0,
                PRIMARY KEY (message_id, chat_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS reactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                username TEXT NOT NULL,
                reaction_type TEXT NOT NULL, -- 'reviewer', 'approval', 'comment'
                UNIQUE(message_id, chat_id, username, reaction_type)
            )",
        )
        .execute(&self.pool)
        .await?;

        // Table to track seen PRs globally to avoid reposting if we restart
        // key: owner/repo#number
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS seen_prs (
                key TEXT PRIMARY KEY,
                seen_at INTEGER NOT NULL
             )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn add_repository(&self, owner: &str, name: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO repositories (owner, name) VALUES (?, ?)")
            .bind(owner)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_repositories(&self) -> Result<Vec<TrackedRepo>> {
        let repos = sqlx::query_as::<_, TrackedRepo>("SELECT * FROM repositories")
            .fetch_all(&self.pool)
            .await?;
        Ok(repos)
    }

    pub async fn save_pr_message(&self, msg: &PrMessage) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO messages 
            (message_id, chat_id, pr_url, title, author, repo_owner, repo_name, pr_number, is_merged, is_draft, re_review_requested)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&msg.message_id)
        .bind(msg.chat_id)
        .bind(&msg.pr_url)
        .bind(&msg.title)
        .bind(&msg.author)
        .bind(&msg.repo_owner)
        .bind(&msg.repo_name)
        .bind(msg.pr_number)
        .bind(msg.is_merged)
        .bind(msg.is_draft)
        .bind(msg.re_review_requested)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_pr_message(
        &self,
        message_id: &str,
        chat_id: i64,
    ) -> Result<Option<PrMessage>> {
        let msg = sqlx::query_as::<_, PrMessage>(
            "SELECT * FROM messages WHERE message_id = ? AND chat_id = ?",
        )
        .bind(message_id)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(msg)
    }

    pub async fn update_reactions(
        &self,
        message_id: &str,
        chat_id: i64,
        reviewers: &[String],
        approvals: &[String],
        comments: &[String],
    ) -> Result<()> {
        // Transactional update
        let mut tx = self.pool.begin().await?;

        // Clear existing for this message
        sqlx::query("DELETE FROM reactions WHERE message_id = ? AND chat_id = ?")
            .bind(message_id)
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;

        for user in reviewers {
            sqlx::query("INSERT INTO reactions (message_id, chat_id, username, reaction_type) VALUES (?, ?, ?, 'reviewer')")
                .bind(message_id).bind(chat_id).bind(user)
                .execute(&mut *tx).await?;
        }
        for user in approvals {
            sqlx::query("INSERT INTO reactions (message_id, chat_id, username, reaction_type) VALUES (?, ?, ?, 'approval')")
                .bind(message_id).bind(chat_id).bind(user)
                .execute(&mut *tx).await?;
        }
        for user in comments {
            sqlx::query("INSERT INTO reactions (message_id, chat_id, username, reaction_type) VALUES (?, ?, ?, 'comment')")
                .bind(message_id).bind(chat_id).bind(user)
                .execute(&mut *tx).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_reactions(
        &self,
        message_id: &str,
        chat_id: i64,
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
        let rows = sqlx::query(
            "SELECT username, reaction_type FROM reactions WHERE message_id = ? AND chat_id = ?",
        )
        .bind(message_id)
        .bind(chat_id)
        .fetch_all(&self.pool)
        .await?;

        let mut reviewers = Vec::new();
        let mut approvals = Vec::new();
        let mut comments = Vec::new();

        for row in rows {
            let username: String = row.get("username");
            let r_type: String = row.get("reaction_type");
            match r_type.as_str() {
                "reviewer" => reviewers.push(username),
                "approval" => approvals.push(username),
                "comment" => comments.push(username),
                _ => {}
            }
        }

        Ok((reviewers, approvals, comments))
    }

    pub async fn is_pr_seen(&self, key: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM seen_prs WHERE key = ?")
            .bind(key)
            .fetch_one(&self.pool)
            .await?;
        Ok(count > 0)
    }

    pub async fn mark_pr_seen(&self, key: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO seen_prs (key, seen_at) VALUES (?, ?)")
            .bind(key)
            .bind(chrono::Utc::now().timestamp())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_all_active_messages(&self) -> Result<Vec<PrMessage>> {
        let msgs = sqlx::query_as::<_, PrMessage>("SELECT * FROM messages WHERE is_merged = 0")
            .fetch_all(&self.pool)
            .await?;
        Ok(msgs)
    }

    pub async fn remove_message(&self, message_id: &str, chat_id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Delete reactions first (FK like behavior)
        sqlx::query("DELETE FROM reactions WHERE message_id = ? AND chat_id = ?")
            .bind(message_id)
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;

        // Delete message
        sqlx::query("DELETE FROM messages WHERE message_id = ? AND chat_id = ?")
            .bind(message_id)
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}
