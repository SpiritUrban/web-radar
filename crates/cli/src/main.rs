//! web-radar — inbound/outbound domain links from Common Crawl web graphs.
//!
//! Two ways to work:
//!
//! * `web-radar index build` once, then `web-radar query example.com` in
//!   milliseconds — see [`web_radar_core::index`];
//! * `web-radar run`, which streams all three files every time and needs no
//!   index (and no spare disk).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use web_radar_core::config::{Config, RankMetric};
use web_radar_core::index::{human_bytes, GraphIndex, Tier, TierState};
use web_radar_core::progress::Progress;
use web_radar_core::query::{Engine, QueryOptions};
use web_radar_core::{meta, scan};

#[derive(Debug, Parser)]
#[command(
    name = "web-radar",
    version,
    about = "Вхідні та вихідні зв'язки доменів із графів Common Crawl",
    long_about = None,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Args, Clone)]
struct CommonArgs {
    /// Шлях до config.toml.
    #[arg(
        short,
        long,
        default_value = "config.toml",
        value_name = "FILE",
        global = true
    )]
    config: PathBuf,

    /// Докладніші логи (`-v` = debug, `-vv` = trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Лише помилки.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Повне сканування всіх файлів для доменів із config.toml (без індексу).
    Run,
    /// Побудова, стан і видалення індексу.
    Index {
        #[command(subcommand)]
        action: IndexAction,
    },
    /// Миттєвий запит по одному домену (потребує індексу).
    Query(QueryArgs),
}

#[derive(Debug, Subcommand)]
enum IndexAction {
    /// Що вже побудовано, скільки це займає і чого бракує.
    Status,
    /// Побудувати рівні індексу.
    Build {
        /// lookup, ranks, inbound або all.
        #[arg(default_value = "all")]
        tiers: Vec<String>,
    },
    /// Видалити рівень індексу і звільнити місце.
    Drop {
        /// lookup, ranks, inbound або all.
        tiers: Vec<String>,
    },
}

#[derive(Debug, Args)]
struct QueryArgs {
    /// Домен у будь-якій формі: example.com або https://www.example.com/page.
    domain: String,

    /// Скільки сусідніх доменів показати в кожному напрямку.
    #[arg(long, default_value_t = 25)]
    top: usize,

    /// pagerank або harmonic (типово — з config.toml).
    #[arg(long)]
    metric: Option<String>,

    /// Вивести повний JSON замість таблиці.
    #[arg(long)]
    json: bool,

    /// Записати результат у results/ як у режимі run.
    #[arg(long)]
    save: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.common.verbose, cli.common.quiet);

    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!();
            eprintln!("Помилка: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(verbose: u8, quiet: bool) {
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level))
        .format_timestamp_secs()
        .init();
}

fn dispatch(cli: &Cli) -> Result<()> {
    let config_path = resolve_config_path(&cli.common.config)?;
    let cfg = Config::load(&config_path)?;
    log::debug!("config: {}", config_path.display());

    match cli.command.as_ref().unwrap_or(&Command::Run) {
        Command::Run => command_run(&cfg),
        Command::Index { action } => match action {
            IndexAction::Status => command_index_status(&cfg),
            IndexAction::Build { tiers } => command_index_build(&cfg, tiers),
            IndexAction::Drop { tiers } => command_index_drop(&cfg, tiers),
        },
        Command::Query(args) => command_query(&cfg, args),
    }
}

fn command_run(cfg: &Config) -> Result<()> {
    println!("web-radar {} — повне сканування", meta::VERSION);
    let (progress, bar) = terminal_progress();
    let written = scan::run(cfg, &progress)?;
    bar.finish_and_clear();

    println!("\nГотово. Результати:");
    for path in &written {
        println!("  • {}", path.display());
    }
    if !cfg.paths.index_dir.join("index.json").is_file() {
        println!(
            "\nПорада: `web-radar index build` перетворює наступні запити з десятків хвилин на мілісекунди."
        );
    }
    Ok(())
}

fn command_index_status(cfg: &Config) -> Result<()> {
    let index = GraphIndex::new(&cfg.paths.index_dir);
    let status = index.status(&cfg.sources());

    println!("Індекс: {}", status.root);
    println!("Вільно на диску: {}", human_bytes(status.free_bytes));
    if status.node_count > 0 {
        println!("Доменів у графі: {}", status.node_count);
    }
    if status.edge_count > 0 {
        println!("Зв'язків у графі: {}", status.edge_count);
    }
    println!();
    for tier in &status.tiers {
        let state = match tier.state {
            TierState::Ready => "готовий",
            TierState::Stale => "застарілий (файли графа змінилися)",
            TierState::Missing => "не побудований",
        };
        let size = if tier.bytes > 0 {
            human_bytes(tier.bytes)
        } else {
            format!("≈{}", human_bytes(tier.estimated_bytes))
        };
        println!("  {:<22} {:<34} {size}", tier.label, state);
        println!("    {}", tier.description);
    }
    if !status.blockers.is_empty() {
        println!("\nПотребує уваги:");
        for blocker in &status.blockers {
            println!("  ! {blocker}");
        }
    }
    // A user who has no data needs the full instruction, not just a complaint.
    if status.sources.iter().any(|source| !source.exists) {
        let sources = cfg.sources();
        println!();
        println!(
            "{}",
            web_radar_core::data_source::instructions(&sources.crawl(), &sources.expected_dir())
        );
    }
    Ok(())
}

fn command_index_build(cfg: &Config, requested: &[String]) -> Result<()> {
    let tiers = parse_tiers(requested)?;
    let index = GraphIndex::new(&cfg.paths.index_dir);
    let sources = cfg.sources();

    let needed: u64 = tiers
        .iter()
        .map(|tier| tier.estimated_bytes(&sources))
        .sum();
    let temp = tiers
        .iter()
        .map(|tier| tier.estimated_temp_bytes(&sources))
        .max()
        .unwrap_or(0);
    println!(
        "Будуємо: {}\nОцінка: {} індексу + до {} тимчасових файлів у {}",
        tiers
            .iter()
            .map(|t| t.label())
            .collect::<Vec<_>>()
            .join(", "),
        human_bytes(needed),
        human_bytes(temp),
        cfg.paths.index_dir.display()
    );

    let (progress, bar) = terminal_progress();
    let meta = index.build(&sources, &tiers, &progress)?;
    bar.finish_and_clear();

    println!(
        "\nГотово. Доменів: {}, зв'язків: {}. Індекс займає {}.",
        meta.node_count,
        meta.edge_count,
        human_bytes(index.status(&sources).total_bytes)
    );
    Ok(())
}

fn command_index_drop(cfg: &Config, requested: &[String]) -> Result<()> {
    let tiers = parse_tiers(requested)?;
    let index = GraphIndex::new(&cfg.paths.index_dir);
    for tier in tiers {
        index.drop_tier(tier)?;
        println!("Видалено рівень «{}»", tier.label());
    }
    Ok(())
}

fn command_query(cfg: &Config, args: &QueryArgs) -> Result<()> {
    let index = GraphIndex::new(&cfg.paths.index_dir);
    let sources = cfg.sources();
    let engine = Engine::open(&index, &sources)?;
    if !engine.capabilities().can_query() {
        bail!(
            "індекс пошуку не побудований — виконайте `web-radar index build lookup`\n\
             (або `web-radar run`, який читає весь граф без індексу)"
        );
    }

    let metric = match args.metric.as_deref() {
        Some(value) => RankMetric::parse(value)?,
        None => cfg.rank_metric,
    };
    let report = engine.query(
        &args.domain,
        QueryOptions {
            metric,
            ..QueryOptions::default()
        },
    )?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report.to_target_result())?
        );
    } else {
        print_report(&report, args.top);
    }
    if args.save {
        let path = engine.write_result(&cfg.paths.results_dir, &report)?;
        println!("\nЗбережено: {}", path.display());
    }
    Ok(())
}

fn print_report(report: &web_radar_core::query::DomainReport, top: usize) {
    println!("\n{}", report.domain);
    if !report.found {
        println!("  не знайдено у цьому випуску графа");
    } else {
        let rank = report
            .rank
            .map(|value| format!("{value:.6e}"))
            .unwrap_or_else(|| "—".into());
        let position = report
            .position
            .map(|value| format!(" (#{value})"))
            .unwrap_or_default();
        println!("  {}: {rank}{position}", report.metric);
        println!(
            "  вхідних: {}   вихідних: {}   за {} мс",
            report.inbound_total, report.outbound_total, report.elapsed_ms
        );
    }
    for (title, entries, total) in [
        (
            "Хто посилається сюди",
            &report.inbound,
            report.inbound_total,
        ),
        ("Куди посилається", &report.outbound, report.outbound_total),
    ] {
        if entries.is_empty() {
            continue;
        }
        println!("\n{title} ({total}):");
        for entry in entries.iter().take(top) {
            println!("  {:<44} {:.4e}", entry.domain, entry.rank);
        }
        if total > top as u64 {
            println!("  … ще {}", total - top as u64);
        }
    }
    for warning in &report.warnings {
        println!("\n! {warning}");
    }
}

fn parse_tiers(requested: &[String]) -> Result<Vec<Tier>> {
    if requested.is_empty() {
        return Ok(Tier::ALL.to_vec());
    }
    let mut tiers = Vec::new();
    for raw in requested {
        if raw.eq_ignore_ascii_case("all") {
            return Ok(Tier::ALL.to_vec());
        }
        match Tier::parse(&raw.to_ascii_lowercase()) {
            Some(tier) if !tiers.contains(&tier) => tiers.push(tier),
            Some(_) => {}
            None => bail!(
                "невідомий рівень «{raw}» — доступні: {}",
                Tier::ALL
                    .iter()
                    .map(|t| t.key())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    Ok(tiers)
}

/// A progress bar wired to the core's progress channel.
fn terminal_progress() -> (Progress, ProgressBar) {
    let bar = ProgressBar::new(1000);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent:>3}%  {msg}",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-"),
    );
    bar.enable_steady_tick(std::time::Duration::from_millis(150));

    let handle = Mutex::new(bar.clone());
    let progress = Progress::new(Arc::new(AtomicBool::new(false)), move |update| {
        let bar = handle.lock().unwrap_or_else(|e| e.into_inner());
        bar.set_position((update.overall * 1000.0) as u64);
        let speed = if update.bytes_per_sec > 0 {
            format!(" · {}/с", human_bytes(update.bytes_per_sec))
        } else {
            String::new()
        };
        let eta = if update.eta_secs > 0 {
            format!(" · залишилось ~{}", format_duration(update.eta_secs))
        } else {
            String::new()
        };
        bar.set_message(format!("{}{speed}{eta}", update.detail));
    });
    (progress, bar)
}

fn format_duration(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds} с"),
        60..=3599 => format!("{} хв", seconds / 60),
        _ => format!("{} год {} хв", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Find `config.toml` next to the cwd, the executable, or the repo root, so a
/// double-clicked binary behaves like one started from the project folder.
fn resolve_config_path(requested: &PathBuf) -> Result<PathBuf> {
    if requested.is_file() {
        return Ok(requested.clone());
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(requested));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(requested));
            // cargo layout: target/release/web-radar → repo root is two up.
            if let Some(root) = dir.parent().and_then(|p| p.parent()) {
                candidates.push(root.join(requested));
            }
        }
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Ok(found);
    }
    bail!(
        "не знайдено файл конфігурації: {}\n\
         Запустіть із теки проєкту або вкажіть повний шлях:\n  web-radar -c /шлях/до/config.toml",
        requested.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tier_names_and_rejects_typos() {
        assert_eq!(parse_tiers(&[]).expect("default"), Tier::ALL.to_vec());
        assert_eq!(
            parse_tiers(&["all".into()]).expect("all"),
            Tier::ALL.to_vec()
        );
        assert_eq!(
            parse_tiers(&["inbound".into(), "Inbound".into()]).expect("dedupes"),
            vec![Tier::Inbound]
        );
        let error = parse_tiers(&["backlinks".into()]).expect_err("typo must fail");
        assert!(
            format!("{error}").contains("inbound"),
            "must list valid names: {error}"
        );
    }

    #[test]
    fn formats_durations_for_humans() {
        assert_eq!(format_duration(45), "45 с");
        assert_eq!(format_duration(600), "10 хв");
        assert_eq!(format_duration(7_800), "2 год 10 хв");
    }

    #[test]
    fn cli_parses_the_shapes_documented_in_the_readme() {
        let cli = Cli::try_parse_from(["web-radar", "query", "example.com", "--top", "5"])
            .expect("query");
        match cli.command {
            Some(Command::Query(args)) => {
                assert_eq!(args.domain, "example.com");
                assert_eq!(args.top, 5);
            }
            other => panic!("expected a query command, got {other:?}"),
        }

        let cli = Cli::try_parse_from(["web-radar", "index", "build", "inbound"]).expect("build");
        assert!(matches!(
            cli.command,
            Some(Command::Index {
                action: IndexAction::Build { .. }
            })
        ));

        // No subcommand keeps the 0.2 behaviour of run.ps1.
        let cli = Cli::try_parse_from(["web-radar", "-c", "testdata/config.toml"]).expect("bare");
        assert!(cli.command.is_none());
        assert_eq!(cli.common.config, PathBuf::from("testdata/config.toml"));
    }
}
