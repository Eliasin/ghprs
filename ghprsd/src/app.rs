use std::{collections::HashMap, ops::DerefMut, sync::Arc};

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::{gh_client::GithubClient, Config};

use ghprs_core::GithubPRStatus;

pub struct Session {
    pub last_new_pr_time: LastNewPrTime,
}

pub struct AppState {
    pub sessions: HashMap<String, Session>,
    pub config: Config,
    pub github_client: GithubClient,
}

type HandlerAppState = State<Arc<Mutex<AppState>>>;

#[derive(Debug)]
pub struct LastNewPrTime(pub Option<DateTime<Utc>>);

pub async fn new_prs_global(state: HandlerAppState) -> Json<Vec<GithubPRStatus>> {
    new_prs(state, Path("global".to_string())).await
}

pub async fn new_prs(
    State(state): HandlerAppState,
    Path(session_name): Path<String>,
) -> Json<Vec<GithubPRStatus>> {
    let mut state = state.lock().await;

    let AppState {
        sessions,
        config,
        github_client,
    } = state.deref_mut();

    let last_new_pr_time: &mut LastNewPrTime = match sessions.get_mut(&session_name) {
        Some(session) => &mut session.last_new_pr_time,
        None => {
            sessions.insert(
                session_name.clone(),
                Session {
                    last_new_pr_time: LastNewPrTime(None),
                },
            );
            &mut sessions
                .get_mut(&session_name)
                .expect("should have just been inserted")
                .last_new_pr_time
        }
    };

    let mut pr_statueses = vec![];

    for repository in config.repositories.iter() {
        let repository_pr_statuses = match github_client
            .new_pr_status(
                repository,
                Some(&config.author),
                last_new_pr_time.0.map(|t| t.into()),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Encountered error processing statuses for repo {} with for author {} since {:?}: {}", repository, config.author, last_new_pr_time, e);
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

    last_new_pr_time.0 = Some(Utc::now());

    Json(pr_statueses)
}
