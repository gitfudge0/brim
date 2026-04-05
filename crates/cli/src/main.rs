use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use brim_storage::config::AppConfig;
use brim_storage::db::Database;
use brim_storage::paths::AppPaths;

mod commands;

#[derive(Parser)]
#[command(name = "brim", about = "Brim — monitor AI assistant quotas")]
#[command(version, long_about = None)]
struct Cli {
    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show usage status for all or a specific provider
    Status {
        /// Provider to query (codex, claude, copilot). Omit for all.
        provider: Option<String>,
        /// Force a fresh fetch (don't use cache)
        #[arg(long)]
        fresh: bool,
    },
    /// Emit machine-readable JSON; compact by default, or current summaries with --full
    Json {
        /// Provider to query (codex, claude, copilot). Omit for all.
        provider: Option<String>,
        /// Force a fresh fetch (don't use cache)
        #[arg(long)]
        fresh: bool,
        /// Emit the current detailed summary JSON shape instead of the compact default
        #[arg(long)]
        full: bool,
    },
    /// Sync usage data from providers
    Sync {
        /// Provider to sync (codex, claude, copilot). Omit for all.
        provider: Option<String>,
    },
    /// Manage authentication for providers
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Show or edit configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show diagnostic information
    Diag,
}

#[derive(Subcommand)]
enum AuthAction {
    /// Show auth status for all providers
    Status,
    /// Set up authentication for a provider
    Login {
        /// Provider to authenticate (codex, claude, copilot)
        provider: String,
    },
    /// Remove stored credentials for a provider
    Logout {
        /// Provider to remove credentials for
        provider: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Initialize default configuration file
    Init,
    /// Open config file in $EDITOR
    Edit,
}

/// Shared application state built once in main.
struct AppState {
    paths: AppPaths,
    config: Arc<AppConfig>,
    db: Arc<Database>,
    http: Arc<reqwest::Client>,
}

impl AppState {
    fn init() -> Result<Self> {
        let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
        paths.ensure_dirs().map_err(|e| anyhow::anyhow!("{}", e))?;

        let config = AppConfig::load(&paths.config_file).map_err(|e| anyhow::anyhow!("{}", e))?;
        let db = Database::open(&paths.db_file).map_err(|e| anyhow::anyhow!("database: {}", e))?;

        let http = reqwest::Client::builder()
            .user_agent("brim/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            paths,
            config: Arc::new(config),
            db: Arc::new(db),
            http: Arc::new(http),
        })
    }

    fn build_sync_engine(&self) -> brim_providers::sync_engine::SyncEngine {
        let registry = brim_providers::registry::build_registry(self.http.clone(), &self.config);
        brim_providers::sync_engine::SyncEngine::new(registry, self.db.clone(), self.config.clone())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up tracing
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .init();

    match cli.command {
        Commands::Status { provider, fresh } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            commands::status::run(&engine, provider, fresh).await?;
        }
        Commands::Json {
            provider,
            fresh,
            full,
        } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            commands::json::run(&engine, provider, fresh, full).await?;
        }
        Commands::Sync { provider } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            commands::sync::run(&engine, provider).await?;
        }
        Commands::Auth { action } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            match action {
                AuthAction::Status => commands::auth::status(&engine).await?,
                AuthAction::Login { provider } => commands::auth::login(&engine, &provider).await?,
                AuthAction::Logout { provider } => commands::auth::logout(&provider).await?,
            }
        }
        Commands::Config { action } => match action {
            ConfigAction::Show => commands::config::show()?,
            ConfigAction::Init => commands::config::init()?,
            ConfigAction::Edit => commands::config::edit()?,
        },
        Commands::Diag => {
            let state = AppState::init()?;
            commands::diag::run(&state.paths, &state.config)?;
        }
    }

    Ok(())
}
