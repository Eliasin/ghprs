use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::process::Stdio;

use thiserror::Error;
use tokio::{process::Command, task::spawn_blocking};

use ghprs_core::GithubPRReview;

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
pub struct GithubPRStatus {
    id: String,
    reviews: Vec<GithubPRReview>,
    title: String,
}

impl GithubPRStatus {
    pub fn convert_to_core(self, repository: String) -> ghprs_core::GithubPRStatus {
        ghprs_core::GithubPRStatus {
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
    pub async fn new_pr_status<S: AsRef<str>>(
        &self,
        repository: S,
        author: Option<S>,
        since: Option<DateTime<chrono::Local>>,
    ) -> Result<Vec<GithubPRStatus>> {
        let mut command = {
            let mut c = Command::new("gh");
            c.arg("pr")
                .arg("list")
                .arg("--repo")
                .arg(repository.as_ref());

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
        let since_timestamp = match since {
            Some(since) => since.timestamp(),
            None => 0,
        };

        let new_prs = spawn_blocking(move || jq_rs::run(&format!(".[] | select(.reviews | map(.submittedAt | fromdate) | max | select(. != null) | . > {})", since_timestamp), pr_json.as_ref())
        ).await.expect("waiting on tokio compute task failed").expect("jq error");

        let pr_status = new_prs
            .split('\n')
            .flat_map(|pr_json| -> Result<GithubPRStatus> {
                serde_json::from_str(pr_json).map_err(|e| GithubClientError::UnexpectedOutput {
                    operation: "gh pr list".to_string(),
                    stderr: String::from_utf8_lossy(&command_output.stderr).to_string(),
                    stdout: String::from_utf8_lossy(&command_output.stdout).to_string(),
                    underlying_error: Box::new(e),
                })
            })
            .collect();

        Ok(pr_status)
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
