use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct GithubAuthor {
    pub login: String,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct GithubPRReview {
    pub id: String,
    pub author: GithubAuthor,
    #[serde(rename = "submittedAt")]
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct GithubPRStatus {
    pub id: String,
    pub reviews: Vec<GithubPRReview>,
    pub title: String,
    pub repository: String,
}

impl GithubPRStatus {
    pub fn latest_review_time(&self) -> Option<DateTime<Utc>> {
        self.reviews.iter().map(|r| r.submitted_at).max()
    }
}
