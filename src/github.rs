use anyhow::Result;
use chrono::{DateTime, Utc};
use octocrab::{models::pulls::PullRequest, Octocrab};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct GithubClient {
    client: Arc<Octocrab>,
    // simple in-memory cache of seen PR IDs to avoid duplicates if we poll frequently
    seen_prs: Arc<Mutex<HashSet<u64>>>,
}

impl GithubClient {
    pub fn new(token: String) -> Result<Self> {
        let client = Octocrab::builder().personal_token(token).build()?;
        Ok(Self {
            client: Arc::new(client),
            seen_prs: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub async fn get_new_prs(
        &self,
        owner: &str,
        repo: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<PullRequest>> {
        let issues = self
            .client
            .pulls(owner, repo)
            .list()
            .sort(octocrab::params::pulls::Sort::Created)
            .direction(octocrab::params::Direction::Descending)
            .state(octocrab::params::State::Open)
            .per_page(10) // fetching few latest
            .send()
            .await?;

        let mut new_prs = Vec::new();
        let mut seen = self.seen_prs.lock().unwrap();

        for pr in issues {
            if let Some(created_at) = pr.created_at {
                if created_at > since && !seen.contains(&pr.id.0) {
                    seen.insert(pr.id.0);
                    new_prs.push(pr);
                }
            }
        }

        Ok(new_prs)
    }

    pub async fn get_pr_details(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<PullRequest> {
        Ok(self.client.pulls(owner, repo).get(pr_number).await?)
    }

    pub async fn get_pr_reviews(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<octocrab::models::pulls::Review>> {
        Ok(self
            .client
            .pulls(owner, repo)
            .list_reviews(pr_number)
            .per_page(100)
            .send()
            .await?
            .take_items())
    }
}
