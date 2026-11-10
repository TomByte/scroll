use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::time::Duration;

use scroll_server::{ConnectError, DbError, Server};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

use scroll_server::config::{self, Config, ConfigError};

const BLOCKING_WORK_GRACE: Duration = Duration::from_secs(1);
const USAGE: &str = "usage: scroll-server [create-invite]";

enum Command {
    Serve,
    /// Prints a sign-up invite code, which is how the first user gets an account.
    CreateInvite,
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error("could not apply database migrations: {0}")]
    Migrate(DbError),
    #[error("could not create an invite: {0}")]
    CreateInvite(DbError),
    #[error("could not listen on {bind_addr}: {source}")]
    Bind {
        bind_addr: String,
        source: io::Error,
    },
    #[error("the server stopped with an error: {0}")]
    Serve(io::Error),
}

fn main() -> ExitCode {
    let (filter, invalid_filter) = match EnvFilter::try_new(config::log_filter()) {
        Ok(filter) => (filter, None),
        Err(error) => (EnvFilter::new(config::DEFAULT_LOG_FILTER), Some(error)),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .init();
    if let Some(error) = invalid_filter {
        tracing::warn!(%error, "RUST_LOG is not a valid log filter; logging at the default level");
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "could not start the async runtime");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run());
    // Work still running on the blocking pool (a password hash, a page merge) gets a moment, not forever.
    runtime.shutdown_timeout(BLOCKING_WORK_GRACE);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "scroll-server failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), StartupError> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match parse_command(&arguments)? {
        Command::Serve => serve(config::from_env()?).await,
        Command::CreateInvite => print_signup_invite(&config::database_url()?).await,
    }
}

fn parse_command(arguments: &[String]) -> Result<Command, StartupError> {
    match arguments {
        [] => Ok(Command::Serve),
        [command] if command == "create-invite" => Ok(Command::CreateInvite),
        _ => Err(StartupError::Usage),
    }
}

async fn serve(config: Config) -> Result<(), StartupError> {
    let pool = scroll_server::connect(&config.database_url)?;
    scroll_server::migrate(&pool)
        .await
        .map_err(StartupError::Migrate)?;
    let bind_addr = config.bind_addr;
    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|source| StartupError::Bind {
            bind_addr: bind_addr.clone(),
            source,
        })?;
    tracing::info!(%bind_addr, "scroll-server listening");
    let server = Server {
        pool,
        settings: config.settings,
    };
    server
        .serve(listener, shutdown_signal())
        .await
        .map_err(StartupError::Serve)?;
    tracing::info!("scroll-server stopped");
    Ok(())
}

async fn print_signup_invite(database_url: &str) -> Result<(), StartupError> {
    let pool = scroll_server::connect(database_url)?;
    scroll_server::migrate(&pool)
        .await
        .map_err(StartupError::Migrate)?;
    let code = scroll_server::create_signup_invite(&pool)
        .await
        .map_err(StartupError::CreateInvite)?;
    println!("{code}");
    Ok(())
}

/// Ctrl-C or SIGTERM. If a signal cannot be watched, the other one still works.
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    let terminate = async {
        match signal(SignalKind::terminate()) {
            Ok(mut terminations) => {
                terminations.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    tracing::info!("shutting down; a second signal stops at once");
    tokio::spawn(async {
        second_signal().await;
        tracing::warn!("stopping at once");
        std::process::exit(1);
    });
}

async fn second_signal() {
    let interrupt = tokio::signal::ctrl_c();
    let Ok(mut terminations) = signal(SignalKind::terminate()) else {
        let _ = interrupt.await;
        return;
    };
    tokio::select! {
        _ = interrupt => {}
        _ = terminations.recv() => {}
    }
}
