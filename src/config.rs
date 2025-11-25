use anyhow::Result;
use dotenv::dotenv;
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub telegram_bot_token: String,
    pub github_token: String,
    pub chat_id: i64,
    pub repositories: Vec<(String, String)>, // (owner, repo)
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv().ok();

        let telegram_bot_token =
            env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN must be set");
        let github_token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN must be set");
        let chat_id = env::var("TELEGRAM_CHAT_ID")
            .expect("TELEGRAM_CHAT_ID must be set")
            .parse::<i64>()
            .expect("TELEGRAM_CHAT_ID must be a valid integer");

        let repositories = env::var("GITHUB_REPOS")
            .map(|repos_str| {
                repos_str
                    .split(',')
                    .map(|s| {
                        let parts: Vec<&str> = s.split('/').collect();
                        if parts.len() != 2 {
                            // Don't panic here, just skip invalid or log
                            eprintln!("Invalid repository format: {}", s);
                            ("".to_string(), "".to_string())
                        } else {
                            (parts[0].to_string(), parts[1].to_string())
                        }
                    })
                    .filter(|(o, r)| !o.is_empty() && !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            telegram_bot_token,
            github_token,
            chat_id,
            repositories,
        })
    }
}
