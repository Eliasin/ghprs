use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use ghprs_core::GithubPRStatus;
use reqwest::blocking::Response;
use tabled::{Table, Tabled};

#[derive(Subcommand, Debug)]
enum PrIdCommand {
    Id {
        #[arg(help = "id of pr")]
        pr_id: String,
    },
    Index {
        #[arg(help = "index/number of pr when listed")]
        index: u32,
    },
}

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
    Ack {
        #[command(subcommand)]
        command: PrIdCommand,
    },
    #[clap(alias = "ua", about = "unacknowledge a review; aliased to 'ua'")]
    Unack {
        #[command(subcommand)]
        command: PrIdCommand,
    },
    #[clap(alias = "cls", about = "clear all session state; aliased to 'cls'")]
    ClearSession {},
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long, default_value_t = 7192)]
    port: u16,

    #[arg(short, long, help = "used to keep track of state kept by server")]
    session_name: String,

    #[command(subcommand)]
    command: Command,
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
                title: format!("{:.20}", pr.title),
                repository: pr.repository.clone(),
                latest_review_time: pr.latest_review_time()?.into(),
            })
        })
        .collect()
}

fn fetch_unacknowledged_prs<S: AsRef<str>>(
    server_url: S,
    session_name: S,
) -> Result<Vec<GithubPRStatus>, Box<dyn std::error::Error>> {
    let session_name = session_name.as_ref();
    let server_url = server_url.as_ref();

    let response =
        reqwest::blocking::get(format!("{server_url}/{session_name}/unacknowledged-prs"))?;

    let mut prs: Vec<GithubPRStatus> = response
        .error_for_status()
        .and_then(
            |response: Response| -> Result<Vec<GithubPRStatus>, reqwest::Error> { response.json() },
        )?
        .into_iter()
        .filter(|pr| !pr.reviews.is_empty())
        .collect();

    prs.sort_by_key(|pr| {
        pr.latest_review_time()
            .expect("already checked that there is at least one element")
    });

    Ok(prs)
}

fn fetch_acknowledged_prs<S: AsRef<str>>(
    server_url: S,
    session_name: S,
) -> Result<Vec<GithubPRStatus>, Box<dyn std::error::Error>> {
    let session_name = session_name.as_ref();
    let server_url = server_url.as_ref();

    let response = reqwest::blocking::get(format!("{server_url}/{session_name}/acknowledgement"))?;

    let mut prs: Vec<GithubPRStatus> = response
        .error_for_status()
        .and_then(
            |response: Response| -> Result<Vec<GithubPRStatus>, reqwest::Error> { response.json() },
        )?
        .into_iter()
        .filter(|pr| !pr.reviews.is_empty())
        .collect();

    prs.sort_by_key(|pr| {
        pr.latest_review_time()
            .expect("already checked that there is at least one element")
    });

    Ok(prs)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let server_url = format!("http://localhost:{}", args.port);
    let session_name = args.session_name;

    match args.command {
        Command::Count {} => match fetch_unacknowledged_prs(&server_url, &session_name) {
            Ok(prs) => {
                println!("{}", prs.len())
            }
            Err(e) => {
                eprintln!("Got error from server: {}", e);
                std::process::exit(1);
            }
        },
        Command::Fetch {} => match fetch_unacknowledged_prs(&server_url, &session_name) {
            Ok(prs) => {
                println!("{}", Table::new(prettyify_prs(&prs)))
            }
            Err(e) => {
                eprintln!("Got error from server: {}", e);
                std::process::exit(1);
            }
        },
        Command::FetchAcked {} => match fetch_acknowledged_prs(&server_url, &session_name) {
            Ok(prs) => {
                println!("{}", Table::new(prettyify_prs(&prs)))
            }
            Err(e) => {
                eprintln!("Got error from server: {}", e);
                std::process::exit(1);
            }
        },
        Command::Ack { command } => {
            let pr_id = match command {
                PrIdCommand::Id { pr_id } => pr_id,
                PrIdCommand::Index { index } => {
                    let index: usize = index.try_into().unwrap();

                    let prs = match fetch_unacknowledged_prs(&server_url, &session_name) {
                        Ok(prs) => prs,
                        Err(e) => {
                            eprintln!("Got error from server: {}", e);
                            std::process::exit(1);
                        }
                    };

                    match prs.get(index) {
                        Some(pr) => {
                            println!("Acknowledging: {}", pr.title);
                            pr.id.clone()
                        }
                        None => {
                            eprintln!(">> ERROR: Invalid index {index}");
                            println!("{}", Table::new(prettyify_prs(&prs)));
                            std::process::exit(1);
                        }
                    }
                }
            };

            let client = reqwest::blocking::Client::new();
            let response = client
                .post(format!(
                    "{server_url}/{session_name}/acknowledgement/{pr_id}"
                ))
                .body("{}")
                .send()?;

            if response.status().is_success() {
                let prs = match fetch_unacknowledged_prs(&server_url, &session_name) {
                    Ok(prs) => prs,
                    Err(e) => {
                        eprintln!("Got error from server: {}", e);
                        std::process::exit(1);
                    }
                };
                println!("Now:\n{}", Table::new(prettyify_prs(&prs)))
            } else {
                eprintln!("Got error from server: {:?}", response.error_for_status());
                std::process::exit(1);
            }
        }
        Command::Unack { command } => {
            let pr_id = match command {
                PrIdCommand::Id { pr_id } => pr_id,
                PrIdCommand::Index { index } => {
                    let index: usize = index.try_into().unwrap();

                    let prs = match fetch_acknowledged_prs(&server_url, &session_name) {
                        Ok(prs) => prs,
                        Err(e) => {
                            eprintln!("Got error from server: {}", e);
                            std::process::exit(1);
                        }
                    };

                    match prs.get(index) {
                        Some(pr) => {
                            println!("Unacknowledging: {}", pr.title);
                            pr.id.clone()
                        }
                        None => {
                            eprintln!(">> ERROR: Invalid index {index}");
                            println!("{}", Table::new(prettyify_prs(&prs)));
                            std::process::exit(1);
                        }
                    }
                }
            };

            let client = reqwest::blocking::Client::new();
            let response = client
                .delete(format!(
                    "{server_url}/{session_name}/acknowledgement/{pr_id}"
                ))
                .body("{}")
                .send()?;

            if response.status().is_success() {
                let prs = match fetch_unacknowledged_prs(&server_url, &session_name) {
                    Ok(prs) => prs,
                    Err(e) => {
                        eprintln!("Got error from server: {}", e);
                        std::process::exit(1);
                    }
                };
                println!("Now:\n{}", Table::new(prettyify_prs(&prs)))
            } else {
                eprintln!("Got error from server: {:?}", response.error_for_status());
                std::process::exit(1);
            }
        }
        Command::ClearSession {} => {
            let client = reqwest::blocking::Client::new();
            let response = client
                .delete(format!("{server_url}/clear-session/{session_name}"))
                .send()?;

            if response.status().is_success() {
                println!("{}", response.text()?);
            } else {
                eprintln!("Got error from server: {:?}", response.error_for_status());
                std::process::exit(1);
            }
        }
    };

    Ok(())
}
