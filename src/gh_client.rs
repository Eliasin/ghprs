use serde::{Deserialize, Serialize};
use std::process::Stdio;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::process::Command;

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

#[derive(Error, Debug)]
pub enum GithubClientError {
    #[error("Cannot find github cli binary in PATH")]
    CannotFindGithubCLI,
    #[error("Not logged into github cli, please use 'gh auth login'")]
    NotLoggedIn,
    #[error(
        "Got unexpected output from operation {operation}, stdout: {stdout}, stderr: {stderr}, underlying error: {underlying_error}"
    )]
    UnexpectedOutput {
        operation: String,
        stderr: String,
        stdout: String,
        underlying_error: Box<dyn std::error::Error>,
    },
    #[error("Got unexpected io error when running {operation}: {underlying_error}")]
    UnexpectedCommandError {
        operation: String,
        underlying_error: std::io::Error,
    },
}

#[derive(Deserialize, Serialize, Debug)]
struct RawGithubPRStatus {
    id: String,
    reviews: Vec<GithubPRReview>,
    title: String,
}

impl GithubPRStatus {
    pub fn convert_to_core(self, repository: String) -> GithubPRStatus {
        GithubPRStatus {
            repository,
            id: self.id,
            reviews: self.reviews,
            title: self.title,
        }
    }
}

pub type Result<T> = std::result::Result<T, GithubClientError>;
pub struct GithubClient {}

impl GithubClient {
    pub async fn new_pr_status<S1: AsRef<str>, S2: AsRef<str>>(
        &self,
        repository: S1,
        author: Option<S2>,
    ) -> Result<Vec<GithubPRStatus>> {
        let repository = repository.as_ref();
        let mut command = {
            let mut c = Command::new("gh");
            c.arg("pr").arg("list").arg("--repo").arg(repository);

            if let Some(author) = author {
                c.arg("--author").arg(author.as_ref());
            }
            c.arg("--json")
                .arg("id,title,reviews")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            c
        };

        let command_output = match command.output().await {
            Ok(command_output) => command_output,
            Err(e) => {
                return Err(GithubClientError::UnexpectedCommandError {
                    operation: "gh pr list".to_string(),
                    underlying_error: e,
                })
            }
        };

        let pr_json = String::from_utf8_lossy(&command_output.stdout).to_string();

        let raw_pr_statuses: Vec<RawGithubPRStatus> =
            serde_json::from_str(&pr_json).map_err(|e| GithubClientError::UnexpectedOutput {
                operation: "gh pr list".to_string(),
                stderr: String::from_utf8_lossy(&command_output.stderr).to_string(),
                stdout: String::from_utf8_lossy(&command_output.stdout).to_string(),
                underlying_error: Box::new(e),
            })?;

        Ok(raw_pr_statuses
            .into_iter()
            .map(|raw| {
                let RawGithubPRStatus { id, reviews, title } = raw;

                GithubPRStatus {
                    repository: repository.to_string(),
                    id,
                    reviews,
                    title,
                }
            })
            .collect())
    }

    pub async fn new() -> Result<GithubClient> {
        match Command::new("gh")
            .arg("auth")
            .arg("status")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(GithubClientError::CannotFindGithubCLI)
            }
            Err(e) => {
                panic!("Got unexpected error checking gh auth status: {e}");
            }
            Ok(status) => match status.code() {
                Some(0) => Ok(GithubClient {}),
                Some(1) => Err(GithubClientError::NotLoggedIn),
                Some(code) => panic!("Got unexpected status code checking gh auth status: {code}"),
                None => panic!("Unexpectedly got no status code checking gh auth status"),
            },
        }
    }
}
