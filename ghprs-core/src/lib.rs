use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct GithubAuthor {
    pub login: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct GithubPRReview {
    pub id: String,
    pub author: GithubAuthor,
    #[serde(rename = "submittedAt")]
    pub submitted_at: DateTime<Utc>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct GithubPRStatus {
    pub id: String,
    pub reviews: Vec<GithubPRReview>,
    pub title: String,
    pub repository: String,
}
