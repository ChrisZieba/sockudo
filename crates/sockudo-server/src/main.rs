#![allow(unused_variables)]
#![allow(dead_code)]
#![allow(unused_assignments)]

mod bootstrap;
pub mod cleanup;
mod history;
mod http_handler;
mod logging;
#[cfg(feature = "mcp")]
mod mcp;
mod middleware;
mod presence_history;
#[cfg(feature = "push")]
mod push_http;
#[cfg(feature = "opentelemetry")]
mod telemetry;
mod ws_handler;

pub use bootstrap::MetricsFactory;
use bootstrap::SockudoServer;

use clap::Parser;
use sockudo_core::error::{Error, Result};
use sockudo_core::options::ServerOptions;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    config: Option<String>,
    /// Rewrite compact append versions of the configured version store as
    /// self-contained records, then exit. Rollback maintenance: stop every
    /// server writing to the store first.
    #[arg(long)]
    materialize_append_storage: bool,
    /// Enable chunked append writes after every writer supports format 2.
    /// Maintenance command; stop writers before changing the store marker.
    #[arg(long, conflicts_with_all = ["disable_append_storage", "materialize_append_storage"])]
    enable_append_storage: bool,
    /// Disable compact writes before materialization and rollback.
    #[arg(long, conflicts_with = "materialize_append_storage")]
    disable_append_storage: bool,
}

// jemalloc is the default allocator, Windows MSVC falls back to the system allocator.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize crypto provider at the very beginning for any TLS usage
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|e| {
            Error::Internal(format!("Failed to install default crypto provider: {e:?}"))
        })?;

    let mut config = ServerOptions::default();
    info!("Starting with default configuration");

    // Try TOML first, fall back to JSON for backward compatibility
    if let Ok(file_config) = ServerOptions::load_from_file("config/config.toml").await {
        config = file_config;
        info!(
            config_path = "config/config.toml",
            "Loaded configuration file"
        );
    } else if let Ok(file_config) = ServerOptions::load_from_file("config/config.json").await {
        config = file_config;
        info!(
            config_path = "config/config.json",
            "Loaded configuration file"
        );
    } else {
        info!("No config/config.toml or config/config.json found. Using defaults.");
    }

    if let Some(config_path) = args.config {
        match ServerOptions::load_from_file(&config_path).await {
            Ok(file_config) => {
                config = file_config;
                info!(config_path = %config_path, "explicit configuration applied");
            }
            Err(error) => {
                error!(config_path = %config_path, error = %error, "explicit configuration load failed, retaining previous");
            }
        }
    }

    match config.override_from_env().await {
        Ok(()) => info!("Applied environment variable overrides"),
        Err(error) => {
            error!(error = %error, "environment variable overrides failed")
        }
    }

    if let Err(e) = config.validate() {
        return Err(Error::ConfigFile(format!(
            "Configuration validation failed: {}",
            e
        )));
    }

    #[cfg(not(feature = "opentelemetry"))]
    if config.opentelemetry.enabled {
        return Err(Error::ConfigFile(
            "OpenTelemetry is enabled in configuration, but this binary was built without the opentelemetry feature"
                .to_string(),
        ));
    }

    #[cfg(feature = "opentelemetry")]
    let telemetry =
        telemetry::Telemetry::initialize(&config.opentelemetry, &config.instance.process_id)
            .map_err(|error| {
                Error::Internal(format!("OpenTelemetry initialization failed: {error}"))
            })?;

    #[cfg(feature = "opentelemetry")]
    let logging_handles = logging::init(&config, &telemetry).map_err(Error::Internal)?;
    #[cfg(not(feature = "opentelemetry"))]
    let logging_handles = logging::init(&config).map_err(Error::Internal)?;

    let resolved_logging = logging::reload(&logging_handles, &config).map_err(Error::Internal)?;
    info!(
        output_format = resolved_logging.output_format,
        filter = %resolved_logging.filter,
        debug = config.debug,
        include_target = resolved_logging.include_target,
        colors_enabled = resolved_logging.colors_enabled,
        source_location = resolved_logging.source_location,
        "logging initialized with resolved configuration"
    );
    if config.logging.is_some() {
        info!("custom logging configuration applied");
    }

    #[cfg(feature = "opentelemetry")]
    {
        let (traces_enabled, metrics_enabled, logs_enabled) = telemetry.enabled_signals();
        info!(
            enabled = config.opentelemetry.enabled,
            traces_enabled, metrics_enabled, logs_enabled, "opentelemetry initialized"
        );
    }

    info!(debug = config.debug, "configuration loading complete");

    let result = if args.materialize_append_storage {
        materialize_append_storage(config).await
    } else if args.enable_append_storage || args.disable_append_storage {
        set_append_storage_enabled(config, args.enable_append_storage).await
    } else {
        run_server(config).await
    };

    #[cfg(feature = "opentelemetry")]
    if let Err(error) = telemetry.shutdown().await {
        error!(error = %error, "opentelemetry shutdown failed");
    }

    result
}

/// Rows per page when materializing compact append storage.
const MATERIALIZE_BATCH_SIZE: usize = 500;

#[cfg(feature = "versioned-messages")]
async fn materialize_append_storage(config: ServerOptions) -> Result<()> {
    let store = history::create_version_store(
        &config.versioned_messages,
        &config.history,
        &config.database,
        &config.database_pooling,
    )
    .await?;
    info!(
        driver = ?config.versioned_messages.driver,
        "materializing compact append storage"
    );
    let rewritten = store
        .materialize_append_storage(MATERIALIZE_BATCH_SIZE)
        .await
        .inspect_err(|error| error!(error = %error, "append storage materialization failed"))?;
    info!(
        rewritten_count = rewritten,
        "append storage materialization complete"
    );
    Ok(())
}

#[cfg(not(feature = "versioned-messages"))]
async fn materialize_append_storage(_config: ServerOptions) -> Result<()> {
    Err(Error::Configuration(
        "append storage materialization requires the versioned-messages feature".to_string(),
    ))
}

#[cfg(feature = "versioned-messages")]
async fn set_append_storage_enabled(config: ServerOptions, enabled: bool) -> Result<()> {
    let store = history::create_version_store(
        &config.versioned_messages,
        &config.history,
        &config.database,
        &config.database_pooling,
    )
    .await?;
    store
        .set_append_storage_enabled(enabled)
        .await
        .inspect_err(|error| error!(error = %error, "append storage marker update failed"))?;
    info!(enabled, "append storage marker updated");
    Ok(())
}

#[cfg(not(feature = "versioned-messages"))]
async fn set_append_storage_enabled(_config: ServerOptions, _enabled: bool) -> Result<()> {
    Err(Error::Configuration(
        "append storage marker requires the versioned-messages feature".to_string(),
    ))
}

async fn run_server(config: ServerOptions) -> Result<()> {
    info!("Starting Sockudo server initialization process with resolved configuration...");

    let server = match SockudoServer::new(config).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "server instance creation failed");
            return Err(e);
        }
    };

    if let Err(e) = server.init().await {
        error!(error = %e, "server components initialization failed");
        return Err(e);
    }

    info!("Starting Sockudo server main services...");
    if let Err(e) = server.start().await {
        error!(error = %e, "server runtime error");
        if let Err(stop_err) = server.stop().await {
            error!(error = %stop_err, "server stop failed after runtime error");
        }
        return Err(e);
    }

    info!("Server main services concluded. Performing final shutdown...");
    if let Err(e) = server.stop().await {
        error!(error = %e, "final server stop failed");
    }

    info!("Sockudo server shutdown complete.");
    Ok(())
}
