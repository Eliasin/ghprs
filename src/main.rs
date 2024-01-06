mod gh_client;
mod prs;

use std::{
    collections::HashSet,
    env,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::bail;
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use gh_client::GithubPRStatus;
use prs::{
    acknowledge_review, clear_session, unacknowledge_review, unacknowledged_prs, Session,
    SessionConfig, SessionState,
};
use serde::Deserialize;
use tabled::{Table, Tabled};

use crate::prs::acknowledged_prs;

#[derive(Subcommand, Debug)]
enum Command {
    #[clap(
        alias = "c",
        about = "counts how many unacknowledged pr reviews there are; aliased to 'c'"
    )]
    Count {},
    #[clap(alias = "f", about = "lists unacknowledged prs; aliased to 'f'")]
    Fetch {},
    #[clap(alias = "fa", about = "lists acknowledged prs; aliased to 'fa'")]
    FetchAcked {},
    #[clap(alias = "a", about = "acknowledge a review; aliased to 'a'")]
    Ack {},
    #[clap(alias = "ua", about = "unacknowledge a review; aliased to 'ua'")]
    Unack {},
    #[clap(alias = "cls", about = "clear all session state; aliased to 'cls'")]
    ClearSession {},
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long, help = "path to config file")]
    session_config_path: Option<PathBuf>,
    #[arg(
        long,
        help = "path to session state, also set by GHPRS_STATE_FILE env variable"
    )]
    session_state_path: Option<PathBuf>,

    #[arg(long, short, default_value_t = false)]
    force: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Deserialize)]
struct Config {
    pub author: String,
    pub repositories: HashSet<String>,
    pub session_state_file: Option<PathBuf>,
}

impl From<Config> for SessionConfig {
    fn from(value: Config) -> Self {
        let Config {
            author,
            repositories,
            session_state_file: _,
        } = value;

        SessionConfig {
            author,
            repositories,
        }
    }
}

fn save_session_config<P: AsRef<Path>>(
    session_config: &SessionConfig,
    session_config_path: P,
) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(session_config_path)?;
    let config_str = toml::to_string(session_config)?;
    file.write_all(config_str.as_bytes())?;

    Ok(())
}

fn save_session_state<P: AsRef<Path>>(
    session_state: &SessionState,
    session_state_path: P,
) -> anyhow::Result<()> {
    let file = std::fs::File::create(session_state_path)?;
    serde_json::to_writer(file, session_state)?;

    Ok(())
}

fn config_directory() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(PathBuf::from(env::var("HOME").ok().unwrap()).join(".config"))
}

const SESSION_CONFIG_FILENAME: &str = "ghprs.toml";
const SESSION_STATE_FILENAME: &str = "ghprs-state.json";

fn save_session(session: &Session, args: &Args) -> anyhow::Result<()> {
    let session_config_path = args
        .session_config_path
        .clone()
        .or(env::var("GHPRS_CONFIG_FILE").ok().map(|s| s.into()))
        .unwrap_or(config_directory().join(SESSION_CONFIG_FILENAME));

    let session_state_path = args
        .session_state_path
        .clone()
        .or(env::var("GHPRS_STATE_FILE").ok().map(|s| s.into()))
        .unwrap_or(config_directory().join(SESSION_STATE_FILENAME));

    let (session_config, session_state): (SessionConfig, SessionState) = session.clone().into();
    if let Err(e) = save_session_config(&session_config, session_config_path) {
        eprintln!("Failed to save session config: {e}");
    };

    if let Err(e) = save_session_state(&session_state, session_state_path) {
        eprintln!("Failed to save session state: {e}");
    };

    Ok(())
}

fn load_session(args: &Args) -> anyhow::Result<Session> {
    let session_config_file_path = args
        .session_config_path
        .clone()
        .or(env::var("GHPRS_CONFIG_FILE").ok().map(|s| s.into()))
        .unwrap_or(config_directory().join(SESSION_CONFIG_FILENAME));

    let Ok(mut config_file) = std::fs::File::open(session_config_file_path) else {
        bail!("Need to provide config file, path is specified in args, as GHPRS_CONFIG_FILE env var or at XDG_CONFIG_HOME/ghprs.toml")
    };
    let mut session_file_contents = String::new();
    if let Err(e) = config_file.read_to_string(&mut session_file_contents) {
        bail!("Failed to read from config file: {e}")
    };

    let config: Config = match toml::from_str(&session_file_contents) {
        Ok(config) => config,
        Err(e) => bail!("Could not parse config: {e}"),
    };

    let session_state_file_path = args
        .session_config_path
        .clone()
        .or(env::var("GHPRS_CONFIG_FILE").ok().map(|s| s.into()))
        .or(config.session_state_file.clone())
        .unwrap_or(config_directory().join(SESSION_STATE_FILENAME));

    let state: SessionState = std::fs::File::open(session_state_file_path)
        .ok()
        .and_then(|file| serde_json::from_reader(file).ok())
        .unwrap_or_default();

    Ok(Session::new(config.into(), state))
}

#[derive(Clone, Debug, Tabled)]
struct PrettyGithubPRStatus {
    pub num: usize,
    pub title: String,
    pub repository: String,
    pub latest_review_time: DateTime<Local>,
}

fn prettyify_prs(prs: &[GithubPRStatus]) -> Vec<PrettyGithubPRStatus> {
    prs.iter()
        .enumerate()
        .filter_map(|(num, pr)| -> Option<PrettyGithubPRStatus> {
            Some(PrettyGithubPRStatus {
                num,
                title: pr.title.clone(),
                repository: pr.repository.clone(),
                latest_review_time: pr.latest_review_time()?.into(),
            })
        })
        .collect()
}

fn select_pr(prs: &[GithubPRStatus]) -> Option<String> {
    if prs.is_empty() {
        println!("{}", Table::new(prettyify_prs(prs)));
        return None;
    }

    let mut buffer = String::new();

    let pr = loop {
        print!("{}\n>> Enter index: ", Table::new(prettyify_prs(prs)));
        std::io::stdout().flush().unwrap();
        io::stdin().read_line(&mut buffer).unwrap();

        match str::parse::<usize>(buffer.trim()) {
            Ok(index) => {
                break match prs.get(index) {
                    Some(pr_id) => pr_id,
                    None => {
                        eprintln!(">> ERROR: Invalid index {index}");
                        continue;
                    }
                }
            }
            Err(e) => {
                eprintln!(">> ERROR: Invalid index: {e}");
                continue;
            }
        };
    };

    println!("Selected '{}'", pr.title);

    Some(pr.id.clone())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    smol::block_on(_main())
}

async fn _main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut session = load_session(&args)?;

    if args.force {
        session.force_update_session_prs();
    }

    match args.command {
        Command::Count {} => {
            println!("{}", unacknowledged_prs(&mut session).await?.len())
        }
        Command::Fetch {} => {
            let prs = unacknowledged_prs(&mut session).await?;
            println!("{}", Table::new(prettyify_prs(&prs)))
        }
        Command::FetchAcked {} => {
            let prs = acknowledged_prs(&mut session).await?;
            println!("{}", Table::new(prettyify_prs(&prs)))
        }
        Command::Ack {} => {
            let prs = unacknowledged_prs(&mut session).await?;

            let pr_id = match select_pr(&prs) {
                Some(pr_id) => pr_id,
                None => {
                    eprintln!("> No prs <");
                    std::process::exit(0);
                }
            };

            match acknowledge_review(&mut session, &pr_id).await {
                Ok(_) => {
                    let prs = unacknowledged_prs(&mut session).await?;
                    println!("\n> Now <\n{}", Table::new(prettyify_prs(&prs)))
                }
                Err(e) => {
                    eprintln!("Got error while acking: {e}");
                }
            }
        }
        Command::Unack {} => {
            let prs = acknowledged_prs(&mut session).await?;

            let pr_id = match select_pr(&prs) {
                Some(pr_id) => pr_id,
                None => {
                    eprintln!("> No prs <");
                    std::process::exit(0);
                }
            };

            match unacknowledge_review(&mut session, &pr_id).await {
                Ok(_) => {
                    let prs = acknowledged_prs(&mut session).await?;
                    println!("\n> Now <\n{}", Table::new(prettyify_prs(&prs)))
                }
                Err(e) => {
                    eprintln!("Got error while unacking: {e}");
                }
            }
        }
        Command::ClearSession {} => {
            clear_session(&mut session).await;
        }
    };

    save_session(&session, &args)?;

    Ok(())
}
