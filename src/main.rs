//! Entry point.
//!
//! Startup is fail-loud: configuration is validated, every log file is opened,
//! and every listener is bound before the proxy serves a single connection. Any
//! failure exits non-zero with a reason rather than starting a partly working
//! proxy.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::{watch, Notify};

use mysql_proxy::config::Config;
use mysql_proxy::logging::writer;
use mysql_proxy::pipeline::{ObserveStage, Pipeline};
use mysql_proxy::proxy::{self, ListenerContext};
use mysql_proxy::row_filter::{RowFilterStage, RuleSet};

const USAGE: &str = "usage: mysql-proxy <config.toml>";

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    }

    match run(PathBuf::from(path)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mysql-proxy: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config_path: PathBuf) -> Result<(), String> {
    let config = Config::load(&config_path).map_err(|e| e.to_string())?;

    let reopen = Arc::new(Notify::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Open every log file and bind every port before serving anything, so a
    // failure on the last listener does not leave the first one accepting.
    let mut prepared = Vec::new();
    let mut writer_tasks = Vec::new();
    for listener_config in &config.listeners {
        let (log, task) = writer::spawn(
            listener_config.name.clone(),
            listener_config.log_file.clone(),
            listener_config.log_channel_capacity,
            Arc::clone(&reopen),
        )
        .await
        .map_err(|e| {
            format!(
                "listener '{}': cannot open log file {}: {e}",
                listener_config.name,
                listener_config.log_file.display()
            )
        })?;
        writer_tasks.push(task);

        let socket = TcpListener::bind(&listener_config.bind).await.map_err(|e| {
            format!(
                "listener '{}': cannot bind {}: {e}",
                listener_config.name, listener_config.bind
            )
        })?;

        // Predicates were already validated when the configuration loaded, so
        // this should not fail; it is checked rather than unwrapped because a
        // bad rule must never reach a client.
        let rules = RuleSet::compile(&listener_config.row_filters).map_err(|(table, e)| {
            format!(
                "listener '{}' row filter for table '{}': {e}",
                listener_config.name, table
            )
        })?;

        // A listener without rules keeps the phase-one pipeline exactly, so it
        // does no statement analysis at all.
        let pipeline = if rules.is_empty() {
            Pipeline::observe_only()
        } else {
            // The observer runs first so the digest describes the statement the
            // client submitted, not the rewritten one.
            Pipeline::new(vec![
                Box::new(ObserveStage),
                Box::new(RowFilterStage::new(rules)),
            ])
        };

        prepared.push((
            socket,
            Arc::new(ListenerContext {
                config: listener_config.clone(),
                log,
                pipeline: Arc::new(pipeline),
            }),
        ));
    }

    let mut serving = Vec::new();
    for (socket, ctx) in prepared {
        eprintln!(
            "listener '{}' proxying {} -> {}, logging to {}{}",
            ctx.config.name,
            ctx.config.bind,
            ctx.config.backend,
            ctx.config.log_file.display(),
            if ctx.config.row_filters.is_empty() {
                String::new()
            } else {
                format!(", row filters on {} table(s)", ctx.config.row_filters.len())
            }
        );
        serving.push(tokio::spawn(proxy::serve(socket, ctx, shutdown_rx.clone())));
    }

    install_signal_handlers(reopen, shutdown_tx).await;

    for task in serving {
        let _ = task.await;
    }
    for task in writer_tasks {
        let _ = task.await;
    }

    Ok(())
}

/// Waits for a termination signal, forwarding hangups to the log writers so an
/// external rotation tool can drive them.
async fn install_signal_handlers(reopen: Arc<Notify>, shutdown: watch::Sender<bool>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot install SIGHUP handler, rotation disabled: {e}");
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown.send(true);
                return;
            }
        };
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot install SIGTERM handler: {e}");
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown.send(true);
                return;
            }
        };

        loop {
            tokio::select! {
                _ = hangup.recv() => {
                    eprintln!("SIGHUP: reopening log files");
                    reopen.notify_waiters();
                }
                _ = term.recv() => break,
                _ = tokio::signal::ctrl_c() => break,
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = reopen;
        let _ = tokio::signal::ctrl_c().await;
    }

    eprintln!("shutting down");
    let _ = shutdown.send(true);
}
