//! Binary entry point.
//!
//! # Startup order is a security property, not tidiness
//!
//! `policy-config` requires that invalid configuration prevents startup, and
//! that the proxy accepts no client connections while it is under-enforcing. So
//! the policy file is loaded and validated **before** the accept socket is
//! bound. If validation fails the process exits and nothing ever listens.
//!
//! The ordering is enforced by the types rather than by comment discipline:
//! [`serve`] takes a [`PolicySet`] **by value**, and the only way to obtain one
//! is a successful [`PolicySet::load_from_path`]. A later reader cannot move the
//! bind above the load without the borrow checker objecting, because there would
//! be no `PolicySet` to hand over yet.
//!
//! # Two configuration files, deliberately
//!
//! The policy file rejects unknown keys and unknown sections, so that a
//! misspelled `[[policy]]` cannot silently yield zero policies and disable all
//! filtering. That rule is load-bearing, which means proxy settings — listen
//! address, frontend address — cannot live in the same file. They are command
//! line arguments instead.

use std::process::ExitCode;
use std::sync::Arc;

use doris_row_filter_proxy::policy::PolicySet;
use doris_row_filter_proxy::session::{PolicyGate, ProxyServer};
use tokio::net::TcpListener;

const USAGE: &str = "\
doris-row-filter-proxy — L7 MySQL proxy enforcing row-level filtering for Apache Doris

USAGE:
    doris-row-filter-proxy --policy <FILE> --listen <ADDR> --backend <ADDR>

OPTIONS:
    --policy <FILE>   Policy configuration (TOML). Validated before anything binds.
    --listen <ADDR>   Address to accept MySQL clients on, e.g. 127.0.0.1:3307
    --backend <ADDR>  Address of the Doris frontend, e.g. 127.0.0.1:9030
    -h, --help        Print this message
";

struct Args {
    policy: String,
    listen: String,
    backend: String,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut policy = None;
    let mut listen = None;
    let mut backend = None;

    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = |name: &str| {
            argv.next()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match flag.as_str() {
            "--policy" => policy = Some(value("--policy")?),
            "--listen" => listen = Some(value("--listen")?),
            "--backend" => backend = Some(value("--backend")?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unrecognised argument {other:?}\n\n{USAGE}")),
        }
    }

    Ok(Args {
        policy: policy.ok_or_else(|| format!("--policy is required\n\n{USAGE}"))?,
        listen: listen.ok_or_else(|| format!("--listen is required\n\n{USAGE}"))?,
        backend: backend.ok_or_else(|| format!("--backend is required\n\n{USAGE}"))?,
    })
}

fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    // STEP 1 — validate configuration. Nothing is listening yet, and if this
    // fails nothing ever will.
    let policies = match PolicySet::load_from_path(&args.policy) {
        Ok(policies) => policies,
        Err(error) => {
            eprintln!("refusing to start: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        path = %args.policy,
        policies = policies.policy_count(),
        "policy configuration validated"
    );

    // STEP 2 — only now may a socket be bound. `serve` consumes the validated
    // set, so this call cannot be hoisted above STEP 1.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("could not start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(serve(args, policies)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("proxy stopped: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Bind and serve. Taking `PolicySet` by value is what makes the startup
/// ordering unforgeable — see the module documentation.
async fn serve(args: Args, policies: PolicySet) -> std::io::Result<()> {
    let listener = TcpListener::bind(&args.listen).await?;
    tracing::info!(
        listen = %args.listen,
        backend = %args.backend,
        "accepting MySQL clients"
    );

    let server = Arc::new(ProxyServer::new(
        args.backend,
        Arc::new(PolicyGate::new(policies)),
    ));
    server.run(listener).await.map_err(std::io::Error::other)
}
