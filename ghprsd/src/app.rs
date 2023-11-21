use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use log::info;
use tokio::sync::Mutex;

use crate::{gh_client::GithubClient, Config};

use ghprs_core::GithubPRStatus;

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct PullRequestId(pub String);

#[derive(Clone, Debug)]
pub struct SessionPr {
    acknowledged: bool,
    pr: GithubPRStatus,
}

impl From<&SessionPr> for GithubPRStatus {
    fn from(value: &SessionPr) -> Self {
        value.pr.clone()
    }
}

#[derive(Default)]
pub struct Session {
    pub prs: HashMap<PullRequestId, SessionPr>,
    pub last_fetch_time: Option<DateTime<Utc>>,
}

pub struct AppState {
    pub sessions: Mutex<HashMap<String, Session>>,
    pub config: Config,
    pub github_client: GithubClient,
}

type HandlerAppState = State<Arc<AppState>>;

#[derive(Debug, Clone, Default)]
pub struct TimeCursor(pub Option<DateTime<Utc>>);

async fn fetch_prs(config: &Config, github_client: &GithubClient) -> Vec<GithubPRStatus> {
    let mut pr_statueses = vec![];

    for repository in config.repositories.iter() {
        let repository_pr_statuses = match github_client
            .new_pr_status(repository, Some(&config.author))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "Encountered error processing statuses for repo {} with for author {}: {}",
                    repository, config.author, e
                );
                continue;
            }
        };

        pr_statueses.extend(
            repository_pr_statuses
                .into_iter()
                .map(|repository_pr_status| {
                    repository_pr_status.convert_to_core(repository.clone())
                }),
        );
    }

    pr_statueses
}

fn update_session_prs(prs: &[GithubPRStatus], session: &mut Session) {
    session.last_fetch_time = Some(Utc::now());

    let mut still_existing_prs = HashSet::new();

    for pr in prs {
        still_existing_prs.insert(pr.id.clone());
        match session.prs.get_mut(&PullRequestId(pr.id.clone())) {
            Some(session_pr) => {
                if pr.latest_review_time() > session_pr.pr.latest_review_time() {
                    session_pr.acknowledged = false;
                }
            }
            None => {
                session.prs.insert(
                    PullRequestId(pr.id.clone()),
                    SessionPr {
                        acknowledged: false,
                        pr: pr.clone(),
                    },
                );
            }
        };
    }

    let session_pr_ids: Vec<PullRequestId> = session.prs.keys().cloned().collect();

    for session_pr_id in session_pr_ids {
        if !still_existing_prs.contains(&session_pr_id.0) {
            session.prs.remove(&session_pr_id);
        }
    }
}

#[axum::debug_handler]
pub async fn unacknowledged_prs(
    State(state): State<Arc<AppState>>,
    Path(session_name): Path<String>,
) -> Json<Vec<GithubPRStatus>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.entry(session_name.clone()).or_default();

    if let Some(last_fetch_time) = session.last_fetch_time {
        if Utc::now().signed_duration_since(last_fetch_time) > Duration::minutes(5) {
            info!("Fetching prs for {session_name} due to timeout from {last_fetch_time}");
            update_session_prs(
                &fetch_prs(&state.config, &state.github_client).await,
                session,
            );
        }
    } else {
        info!("Fetching prs for new session {session_name}");
        update_session_prs(
            &fetch_prs(&state.config, &state.github_client).await,
            session,
        );
    }

    let prs = session
        .prs
        .iter()
        .filter_map(|(_, pr)| -> Option<GithubPRStatus> {
            if !pr.acknowledged {
                Some(pr.into())
            } else {
                None
            }
        })
        .collect::<Vec<GithubPRStatus>>();

    Json(prs)
}

pub async fn acknowledge_review(
    State(state): State<Arc<AppState>>,
    Path((session_name, pr_id)): Path<(String, String)>,
) -> StatusCode {
    let mut sessions = state.sessions.lock().await;

    let session = sessions.entry(session_name.clone()).or_default();

    if let Some(last_fetch_time) = session.last_fetch_time {
        if Utc::now().signed_duration_since(last_fetch_time) > Duration::minutes(5) {
            info!("Fetching prs for {session_name} due to timeout from {last_fetch_time}");
            update_session_prs(
                &fetch_prs(&state.config, &state.github_client).await,
                session,
            );
        }
    } else {
        info!("Fetching prs for new session {session_name}");
        update_session_prs(
            &fetch_prs(&state.config, &state.github_client).await,
            session,
        );
    }

    match session.prs.get_mut(&PullRequestId(pr_id.clone())) {
        Some(pr) => {
            info!("Acked pr reviews for session {session_name} pr {pr_id}");
            pr.acknowledged = true;
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn unacknowledge_review(
    State(state): State<Arc<AppState>>,
    Path((session_name, pr_id)): Path<(String, String)>,
) -> StatusCode {
    let mut sessions = state.sessions.lock().await;

    let session = sessions.entry(session_name.clone()).or_default();

    if let Some(last_fetch_time) = session.last_fetch_time {
        if Utc::now().signed_duration_since(last_fetch_time) > Duration::minutes(5) {
            info!("Fetching prs for {session_name} due to timeout from {last_fetch_time}");
            update_session_prs(
                &fetch_prs(&state.config, &state.github_client).await,
                session,
            );
        }
    } else {
        info!("Fetching prs for new session {session_name}");
        update_session_prs(
            &fetch_prs(&state.config, &state.github_client).await,
            session,
        );
    }

    match session.prs.get_mut(&PullRequestId(pr_id.clone())) {
        Some(pr) => {
            info!("Unacked pr reviews for session {session_name} pr {pr_id}");
            pr.acknowledged = false;
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn acknowledged_reviews(
    State(state): State<Arc<AppState>>,
    Path(session_name): Path<String>,
) -> Json<Vec<GithubPRStatus>> {
    let mut sessions = state.sessions.lock().await;

    let prs = sessions
        .entry(session_name)
        .or_default()
        .prs
        .iter()
        .filter_map(|(_, pr)| -> Option<GithubPRStatus> {
            if pr.acknowledged {
                Some(pr.into())
            } else {
                None
            }
        })
        .collect::<Vec<GithubPRStatus>>();

    Json(prs)
}

pub async fn clear_session(
    State(state): HandlerAppState,
    Path(session_name): Path<String>,
) -> StatusCode {
    let mut sessions = state.sessions.lock().await;

    match sessions.remove(&session_name) {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}
