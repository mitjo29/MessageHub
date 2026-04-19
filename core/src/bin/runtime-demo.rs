//! runtime-demo — smoke-test harness for the Plan 6 Runtime.
//!
//! Reads `messagehub.toml`, opens a SQLCipher store, runs the Runtime
//! against real email + Telegram accounts, prints events to stdout,
//! exits cleanly on Ctrl-C. See
//! `docs/superpowers/specs/2026-04-19-plan7a-runtime-demo-design.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    init_tracing();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("runtime-demo: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_target(false).init();
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Subsequent tasks replace this body with the full Runtime flow.
    eprintln!("runtime-demo: scaffold — real flow lands in Task 3+");
    Ok(())
}
