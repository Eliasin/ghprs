use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::{
    gh_client::{GithubClient, GithubClientError},
    GithubPRStatus,
};
use chrono::{DateTime, Duration, Utc};

pub type PullRequestId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionPr {
    acknowledged: bool,
    pr: GithubPRStatus,
}

impl From<&SessionPr> for GithubPRStatus {
    fn from(value: &SessionPr) -> Self {
        value.pr.clone()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub author: String,
    pub repositories: HashSet<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionState {
    pub last_fetch_time: Option<DateTime<Utc>>,
    pub prs: HashMap<PullRequestId, SessionPr>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Session {
    pub prs: HashMap<PullRequestId, SessionPr>,
    pub author: String,
    pub repositories: HashSet<String>,
    pub last_fetch_time: Option<DateTime<Utc>>,
}

impl From<Session> for (SessionConfig, SessionState) {
    fn from(value: Session) -> Self {
        let Session {
            prs,
            author,
            repositories,
            last_fetch_time,
        } = value;
        (
            SessionConfig {
                author,
                repositories,
            },
            SessionState {
                last_fetch_time,
                prs,
            },
        )
    }
}

impl Session {
    pub fn new(config: SessionConfig, state: SessionState) -> Session {
        let SessionConfig {
            author,
            repositories,
        } = config;
        let SessionState {
            last_fetch_time,
            prs,
        } = state;

        Session {
            author,
            repositories,
            last_fetch_time,
            prs,
        }
    }
}

impl Session {
    pub async fn fetch_prs(&self, github_client: &GithubClient) -> Vec<GithubPRStatus> {
        use futures::future::join_all;
        let Session {
            prs: _,
            author,
            repositories,
            last_fetch_time: _,
        } = self;

        let pr_statuses: Vec<Option<Vec<GithubPRStatus>>> =
            join_all(repositories.iter().map(|repository| async move {
                let repository_pr_statuses =
                    match github_client.new_pr_status(repository, Some(author)).await {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!(
                        "Encountered error processing statuses for repo {} with for author {}: {}",
                        &repository, author, e
                    );
                            return None;
                        }
                    };

                Some(
                    repository_pr_statuses
                        .into_iter()
                        .map(|repository_pr_status| {
                            repository_pr_status.convert_to_core(repository.clone())
                        })
                        .collect(),
                )
            }))
            .await;

        pr_statuses
            .into_iter()
            .flat_map(|p| p.into_iter().flatten())
            .collect()
    }

    pub fn force_update_session_prs(&mut self) {
        self.last_fetch_time = None;
    }

    pub async fn update_session_prs(&mut self) -> Result<(), GithubClientError> {
        if let Some(last_fetch_time) = self.last_fetch_time {
            let time_since_last_fetch = Utc::now().signed_duration_since(last_fetch_time);
            if time_since_last_fetch < Duration::minutes(5) {
                return Ok(());
            }
        }

        let gh_client = GithubClient::new().await?;
        let prs = self.fetch_prs(&gh_client).await;
        self.last_fetch_time = Some(Utc::now());

        let mut still_existing_prs = HashSet::new();

        for pr in prs {
            still_existing_prs.insert(pr.id.clone());
            match self.prs.get_mut(&pr.id) {
                Some(session_pr) => {
                    if let Some(incoming_latest_review_time) = pr.latest_review_time() {
                        let session_pr_latest_review_time = session_pr.pr.latest_review_time();

                        let incoming_has_new_review = session_pr_latest_review_time
                            .map(|session_latest_review_time| {
                                incoming_latest_review_time > session_latest_review_time
                            })
                            .unwrap_or(true);

                        if incoming_has_new_review {
                            session_pr.acknowledged = false;
                        }
                    }

                    session_pr.pr = pr.clone();
                }
                None => {
                    self.prs.insert(
                        pr.id.clone(),
                        SessionPr {
                            acknowledged: false,
                            pr: pr.clone(),
                        },
                    );
                }
            };
        }

        let session_pr_ids: Vec<PullRequestId> = self.prs.keys().cloned().collect();

        for session_pr_id in session_pr_ids {
            if !still_existing_prs.contains(&session_pr_id) {
                self.prs.remove(&session_pr_id);
            }
        }

        Ok(())
    }
}

pub async fn unacknowledged_prs(
    session: &mut Session,
) -> Result<Vec<GithubPRStatus>, GithubClientError> {
    session.update_session_prs().await?;

    let prs = session
        .prs
        .iter()
        .filter_map(|(_, pr)| -> Option<GithubPRStatus> {
            if !pr.acknowledged && !pr.pr.reviews.is_empty() {
                Some(pr.into())
            } else {
                None
            }
        })
        .collect::<Vec<GithubPRStatus>>();

    Ok(prs)
}

pub async fn acknowledge_review(
    session: &mut Session,
    pr_id: &PullRequestId,
) -> anyhow::Result<()> {
    session.update_session_prs().await?;

    match session.prs.get_mut(pr_id) {
        Some(pr) => {
            pr.acknowledged = true;
            Ok(())
        }
        None => Err(anyhow!("Could not find PR with ID: {pr_id}")),
    }
}

pub async fn unacknowledge_review(
    session: &mut Session,
    pr_id: &PullRequestId,
) -> anyhow::Result<()> {
    session.update_session_prs().await?;

    match session.prs.get_mut(pr_id) {
        Some(pr) => {
            pr.acknowledged = false;
            Ok(())
        }
        None => Err(anyhow!("Could not find PR with ID: {pr_id}")),
    }
}

pub async fn acknowledged_prs(
    session: &mut Session,
) -> Result<Vec<GithubPRStatus>, GithubClientError> {
    session.update_session_prs().await?;

    Ok(session
        .prs
        .iter()
        .filter_map(|(_, pr)| -> Option<GithubPRStatus> {
            if pr.acknowledged {
                Some(pr.into())
            } else {
                None
            }
        })
        .collect::<Vec<GithubPRStatus>>())
}

pub async fn clear_session(session: &mut Session) {
    session.prs.clear();
}
