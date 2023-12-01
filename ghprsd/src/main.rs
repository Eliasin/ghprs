mod app;
mod gh_client;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app::{AppState, Session};
use axum::routing::{delete, get, post};
use axum::Router;
use clap::Parser;
use gh_client::GithubClient;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    config: Option<String>,
}

#[derive(Deserialize)]
pub struct Config {
    author: String,
    repositories: Vec<String>,
    port: Option<u16>,
    session_file_path: Option<PathBuf>,
}

const DEFAULT_PORT: u16 = 7192;

async fn get_config(args: Args) -> Result<Config, Box<dyn std::error::Error>> {
    let mut config_file = File::open(args.config.unwrap_or("config.toml".to_string())).await?;
    let mut config_file_contents = vec![];
    config_file.read_to_end(&mut config_file_contents).await?;

    Ok(toml::from_str(
        String::from_utf8_lossy(&config_file_contents).as_ref(),
    )?)
}

pub fn save_sessions<P: AsRef<Path>>(
    session_file_path: Option<&P>,
    sessions: &HashMap<String, Session>,
) {
    let Some(session_file_path) = session_file_path else {
        return;
    };
    let session_file_path = session_file_path.as_ref();

    let Ok(file) = std::fs::File::create(session_file_path) else {
        log::error!(
            "Failed to sync sessions to disk at {}",
            session_file_path.display()
        );
        return;
    };

    if let Err(e) = serde_json::to_writer(file, sessions) {
        log::error!(
            "Failed to serialize/write sessions to disk at {}, error: {e}",
            session_file_path.display()
        )
    }
}

fn load_sessions<P: AsRef<Path>>(session_file_path: Option<&P>) -> HashMap<String, Session> {
    let Some(session_file_path) = session_file_path else {
        return HashMap::new();
    };
    let session_file_path = session_file_path.as_ref();

    let Ok(file) = std::fs::File::open(session_file_path) else {
        log::warn!("No file found at session file path provided, using empty initializer");
        return HashMap::new();
    };

    serde_json::from_reader(file).unwrap_or_else(|_| {
        panic!(
            "could not parse file at {} as JSON",
            session_file_path.display()
        )
    })
}

async fn serve(config: Config, github_client: GithubClient) {
    let port = config.port;

    let sessions = Mutex::new(load_sessions(config.session_file_path.as_ref()));

    let app_state = Arc::new(AppState {
        config,
        github_client,
        sessions,
    });

    let app = Router::new()
        .route(
            "/:session_name/unacknowledged-prs",
            get(app::unacknowledged_prs),
        )
        .route(
            "/:session_name/acknowledgement/:pr_id",
            post(app::acknowledge_review).delete(app::unacknowledge_review),
        )
        .route(
            "/:session_name/acknowledgement",
            get(app::acknowledged_reviews),
        )
        .route("/:session_name/clear-session", delete(app::clear_session))
        .with_state(app_state);

    axum::Server::bind(
        &format!("127.0.0.1:{}", port.unwrap_or(DEFAULT_PORT))
            .parse()
            .expect("invalid host address"),
    )
    .serve(app.into_make_service())
    .await
    .expect("failed to start axum service");
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_env().unwrap();
    let config = get_config(Args::parse()).await?;
    let github_client = GithubClient::new().await?;

    serve(config, github_client).await;

    Ok(())
}
