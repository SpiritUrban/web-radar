//! web-radar — extract inbound domain links from Common Crawl web graphs.
//!
//! Streams multi-GB vertices / edges / ranks files with low RAM usage and
//! writes one JSON result file per configured target domain.

mod config;
mod processor;
mod reverse;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use log::error;

use config::Config;

/// Memory-efficient CLI for Common Crawl domain-level web graphs.
///
/// For each target domain listed in the config, finds every domain that
/// links *to* it and writes `results/{reversed-domain}.json`.
#[derive(Debug, Parser)]
#[command(
    name = "web-radar",
    version,
    about = "Extract inbound links for target domains from Common Crawl domain web graphs",
    long_about = None
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "config.toml", value_name = "FILE")]
    config: PathBuf,

    /// Increase log verbosity (`-v` = info, `-vv` = debug, `-vvv` = trace).
    /// Default without flags is `info`.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Silence all logs except errors.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,
}

fn init_logging(verbose: u8, quiet: bool) {
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(level),
    )
    .format_timestamp_secs()
    .init();
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);

    if let Err(err) = try_main(&cli) {
        // Print the full error chain.
        error!("{err:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn try_main(cli: &Cli) -> Result<()> {
    log::info!("web-radar {}", env!("CARGO_PKG_VERSION"));
    log::info!("loading config from {}", cli.config.display());

    let cfg = Config::load(&cli.config)?;
    processor::run(&cfg)?;
    Ok(())
}
