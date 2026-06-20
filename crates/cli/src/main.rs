use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use brim_storage::config::AppConfig;
use brim_storage::db::Database;
use brim_storage::paths::AppPaths;

mod commands;

const STATUS_AFTER_HELP: &str = "Examples:\n  brim status\n  brim status claude --fresh";
const JSON_AFTER_HELP: &str = "Examples:\n  brim json\n  brim json codex --full";
const AUTH_LOGIN_AFTER_HELP: &str = "Examples:\n  brim auth login copilot";
const CONFIG_INIT_AFTER_HELP: &str = "Examples:\n  brim config init";

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
    #[command(after_help = STATUS_AFTER_HELP)]
    Status {
        /// Provider to query (codex, claude, copilot). Omit for all supported providers.
        provider: Option<String>,
        /// Force a fresh fetch (don't use cache)
        #[arg(long)]
        fresh: bool,
    },
    /// Emit machine-readable JSON; compact by default, or current summaries with --full
    #[command(after_help = JSON_AFTER_HELP)]
    Json {
        /// Provider to query (codex, claude, copilot). Omit for all providers currently included in usage output.
        provider: Option<String>,
        /// Force a fresh fetch (don't use cache)
        #[arg(long)]
        fresh: bool,
        /// Emit the current detailed summary JSON shape instead of the compact default
        #[arg(long)]
        full: bool,
        /// Include history metrics in the JSON output
        #[arg(long)]
        history: bool,
    },
    /// Show usage history, burn rate, and trends
    History {
        /// Provider to show history for (codex, claude, copilot). Omit for all.
        provider: Option<String>,
        /// Number of days of history to include
        #[arg(long, default_value_t = 60)]
        days: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Sync usage data from providers
    Sync {
        /// Provider to sync (codex, claude, copilot). Omit for all enabled providers.
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
    /// Remove the locally installed brim binary (keeps config, state, and credentials)
    Uninstall,
}

#[derive(Subcommand)]
enum AuthAction {
    /// Show auth status for all supported providers
    Status,
    /// Set up authentication for a provider
    #[command(after_help = AUTH_LOGIN_AFTER_HELP)]
    Login {
        /// Provider to authenticate (codex, claude, copilot)
        provider: String,
    },
    /// Remove brim-managed credentials for a provider, or explain manual logout for provider-managed auth
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
    #[command(after_help = CONFIG_INIT_AFTER_HELP)]
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
            history,
        } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            commands::json::run(&engine, provider, fresh, full, history).await?;
        }
        Commands::History {
            provider,
            days,
            json,
        } => {
            let state = AppState::init()?;
            let engine = state.build_sync_engine();
            commands::history::run(&engine, provider, days, json).await?;
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
        Commands::Uninstall => {
            commands::uninstall::run()?;
        }
    }

    Ok(())
}
