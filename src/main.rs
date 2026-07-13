//! web-radar — extract inbound/outbound domain links from Common Crawl web graphs.
//!
//! Streams multi-GB vertices / edges / ranks files with low RAM usage and
//! writes one JSON result file per configured target domain (own rank +
//! who links to it and where it links to).

mod config;
mod processor;
mod reverse;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::Parser;
use log::error;

use config::Config;

/// Memory-efficient CLI for Common Crawl domain-level web graphs.
///
/// For each target domain listed in the config: reports its own rank,
/// every domain that links *to* it (inbound), and every domain it links
/// *to* (outbound). Writes `results/{reversed-domain}.json`.
#[derive(Debug, Parser)]
#[command(
    name = "web-radar",
    version,
    about = "Extract inbound/outbound links and ranks for target domains from Common Crawl domain web graphs",
    long_about = None
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "config.toml", value_name = "FILE")]
    config: PathBuf,

    /// Increase log verbosity (`-v` = info, `-vv` = debug, `-vvv` = trace).
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
            0 | 1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level))
        .format_timestamp_secs()
        .init();
}

/// Prefer the directory of the executable's config lookup: if user double-clicks
/// the exe, cwd may be wrong — also try next to the exe and the project root.
fn resolve_config_path(requested: &PathBuf) -> Result<PathBuf> {
    if requested.is_file() {
        return Ok(requested.clone());
    }

    // relative to cwd
    if let Ok(cwd) = env::current_dir() {
        let p = cwd.join(requested);
        if p.is_file() {
            return Ok(p);
        }
    }

    // next to the executable (target/release/web-radar.exe → ../../config.toml also tried)
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(requested);
            if p.is_file() {
                return Ok(p);
            }
            // cargo layout: target/release/web-radar.exe → repo root is ../..
            if let Some(repo) = dir.parent().and_then(|p| p.parent()) {
                let p = repo.join(requested);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }

    bail!(
        "config file not found: {}\n\
         Run from the project folder, or pass full path:\n\
           web-radar.exe -c C:\\path\\to\\config.toml\n\
         Easiest:  .\\run.ps1",
        requested.display()
    )
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);

    if let Err(err) = try_main(&cli) {
        error!("{err:#}");
        eprintln!();
        eprintln!("Hint: from project root run:  .\\run.ps1");
        eprintln!("      quick demo (no big downloads):  .\\run.ps1 -Demo");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn try_main(cli: &Cli) -> Result<()> {
    log::info!("web-radar {}", env!("CARGO_PKG_VERSION"));

    let config_path = resolve_config_path(&cli.config)?;
    log::info!("loading config from {}", config_path.display());

    let cfg = Config::load(&config_path)?;
    log::info!("results will be written to {}", cfg.paths.results_dir.display());
    log::info!("vertices: {}", cfg.paths.vertices.display());
    log::info!("edges:    {}", cfg.paths.edges.display());
    log::info!("ranks:    {}", cfg.paths.ranks.display());

    processor::run(&cfg)?;
    Ok(())
}
