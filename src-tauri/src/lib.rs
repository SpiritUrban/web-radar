//! Desktop shell around [`web_radar_core`].
//!
//! Everything expensive happens on a blocking thread and reports through the
//! `job-progress` event; every long job can be cancelled from the UI.

mod db;
mod jobs;
mod manual_audit;
mod seo;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use web_radar_core::config::{Config, PathsConfig, RankMetric, Target};
use web_radar_core::index::{GraphIndex, IndexStatus, Tier};
use web_radar_core::query::{Capabilities, DomainReport, Engine, QueryOptions};
use web_radar_core::{meta, scan};

use db::{RunRecord, SeoReportRecord};
use jobs::JobManager;

pub struct AppState {
    db: Mutex<rusqlite::Connection>,
    config_path: Mutex<PathBuf>,
    jobs: JobManager,
}

/// The config as the UI edits it: flat, camelCase, all paths as strings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    vertices: String,
    edges: String,
    ranks: String,
    results_dir: String,
    index_dir: String,
    rank_metric: String,
    targets: Vec<String>,
}

impl UiConfig {
    fn from_config(cfg: &Config) -> Self {
        Self {
            vertices: path_string(&cfg.paths.vertices),
            edges: path_string(&cfg.paths.edges),
            ranks: path_string(&cfg.paths.ranks),
            results_dir: path_string(&cfg.paths.results_dir),
            index_dir: path_string(&cfg.paths.index_dir),
            rank_metric: cfg.rank_metric.as_str().to_string(),
            targets: cfg.targets.iter().map(|t| t.domain.clone()).collect(),
        }
    }

    fn into_engine_config(self) -> Result<Config, String> {
        let rank_metric = RankMetric::parse(&self.rank_metric).map_err(|e| e.to_string())?;
        Ok(Config {
            paths: PathsConfig {
                vertices: PathBuf::from(self.vertices),
                edges: PathBuf::from(self.edges),
                ranks: PathBuf::from(self.ranks),
                results_dir: PathBuf::from(self.results_dir),
                index_dir: PathBuf::from(self.index_dir),
            },
            rank_metric,
            targets: self
                .targets
                .into_iter()
                .filter(|domain| !domain.trim().is_empty())
                .map(|domain| Target { domain })
                .collect(),
            config_dir: PathBuf::new(),
        })
    }
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Author and product strings, so the UI never hardcodes them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductMetadata {
    product_name: &'static str,
    version: &'static str,
    author: &'static str,
    author_url: &'static str,
    author_github_url: &'static str,
    repository_url: &'static str,
    site_url: &'static str,
    copyright: &'static str,
    data_source_url: &'static str,
}

#[tauri::command]
fn product_metadata() -> ProductMetadata {
    ProductMetadata {
        product_name: meta::PRODUCT_NAME,
        version: meta::VERSION,
        author: meta::AUTHOR,
        author_url: meta::AUTHOR_URL,
        author_github_url: meta::AUTHOR_GITHUB_URL,
        repository_url: meta::REPOSITORY_URL,
        site_url: meta::SITE_URL,
        copyright: meta::COPYRIGHT,
        data_source_url: meta::DATA_SOURCE_URL,
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Look for `config.toml` next to the executable, the cwd, or the bundle.
fn locate_config(app: &tauri::App) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut candidates = vec![cwd.join("config.toml")];
    if let Some(parent) = cwd.parent() {
        candidates.push(parent.join("config.toml"));
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("config.toml"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.toml"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| cwd.join("config.toml"))
}

fn load_config(state: &State<'_, AppState>) -> Result<Config, String> {
    let path = state.config_path.lock().map_err(|_| "config lock")?.clone();
    Config::load(&path).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn load_default_config(state: State<'_, AppState>) -> Result<UiConfig, String> {
    load_config(&state).map(|cfg| UiConfig::from_config(&cfg))
}

/// Persist the edited config back to `config.toml`.
#[tauri::command]
fn save_config(state: State<'_, AppState>, config: UiConfig) -> Result<String, String> {
    let cfg = config.into_engine_config()?;
    let path = state.config_path.lock().map_err(|_| "config lock")?.clone();
    let toml = toml_for(&cfg);
    std::fs::write(&path, toml)
        .map_err(|error| format!("не вдалося записати {}: {error}", path.display()))?;
    Ok(path_string(&path))
}

/// Hand-written so the file keeps its comments' spirit and stable key order.
fn toml_for(cfg: &Config) -> String {
    // Top-level keys must precede the first table header: a `rank_metric` line
    // written after `[paths]` becomes `paths.rank_metric` and is silently lost.
    let mut out = format!(
        "# === web-radar ===\n# Paths are relative to THIS file.\n\nrank_metric = \"{}\"\n\n[paths]\n",
        cfg.rank_metric.as_str()
    );
    for (key, value) in [
        ("vertices", &cfg.paths.vertices),
        ("edges", &cfg.paths.edges),
        ("ranks", &cfg.paths.ranks),
        ("results_dir", &cfg.paths.results_dir),
        ("index_dir", &cfg.paths.index_dir),
    ] {
        out.push_str(&format!("{key} = {}\n", toml_string(&path_string(value))));
    }
    for target in &cfg.targets {
        out.push_str(&format!(
            "\n[[targets]]\ndomain = {}\n",
            toml_string(&target.domain)
        ));
    }
    out
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

fn index_and_sources(
    config: UiConfig,
) -> Result<(GraphIndex, web_radar_core::index::GraphSources, Config), String> {
    let cfg = config.into_engine_config()?;
    let index = GraphIndex::new(&cfg.paths.index_dir);
    let sources = cfg.sources();
    Ok((index, sources, cfg))
}

#[tauri::command]
fn index_status(config: UiConfig) -> Result<IndexStatus, String> {
    let (index, sources, _) = index_and_sources(config)?;
    Ok(index.status(&sources))
}

#[tauri::command]
async fn build_index(
    app: AppHandle,
    state: State<'_, AppState>,
    config: UiConfig,
    tiers: Vec<String>,
) -> Result<IndexStatus, String> {
    // Two builds writing the same files would corrupt both.
    if state.jobs.is_running() {
        return Err(
            "інша операція вже виконується — зупиніть її або дочекайтесь завершення".into(),
        );
    }
    let (index, sources, cfg) = index_and_sources(config)?;
    let wanted: Vec<Tier> = tiers
        .iter()
        .filter_map(|key| Tier::parse(key))
        .collect::<Vec<_>>();
    if wanted.is_empty() {
        return Err("не вибрано жодного рівня індексу".into());
    }

    let run_id = {
        let conn = state.db.lock().map_err(|_| "DB lock")?;
        db::start_run(
            &conn,
            "index",
            cfg.rank_metric.as_str(),
            &[],
            &path_string(&cfg.paths.index_dir),
        )
        .map_err(|e| e.to_string())?
    };
    let progress = state.jobs.begin(&app, run_id, "index");
    let handle = state.jobs.clone_handle();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let outcome = index.build(&sources, &wanted, &progress);
        let status = index.status(&sources);
        outcome.map(|_| status)
    })
    .await
    .map_err(|error| error.to_string())?;

    handle.finish();
    let error = result.as_ref().err().map(|error| format!("{error:#}"));
    {
        let conn = state.db.lock().map_err(|_| "DB lock")?;
        db::finish_run(&conn, run_id, error.as_deref()).map_err(|e| e.to_string())?;
    }
    result.map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn drop_index_tier(config: UiConfig, tier: String) -> Result<IndexStatus, String> {
    let (index, sources, _) = index_and_sources(config)?;
    let tier = Tier::parse(&tier).ok_or_else(|| format!("невідомий рівень «{tier}»"))?;
    index
        .drop_tier(tier)
        .map_err(|error| format!("{error:#}"))?;
    Ok(index.status(&sources))
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>) -> Result<(), String> {
    state.jobs.cancel();
    Ok(())
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutcome {
    report: DomainReport,
    capabilities: Capabilities,
    node_count: u64,
}

#[tauri::command]
async fn query_domain(
    config: UiConfig,
    domain: String,
    limit: Option<usize>,
) -> Result<QueryOutcome, String> {
    let (index, sources, cfg) = index_and_sources(config)?;
    tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::open(&index, &sources).map_err(|error| format!("{error:#}"))?;
        let options = QueryOptions {
            metric: cfg.rank_metric,
            limit: limit.unwrap_or(web_radar_core::query::DEFAULT_LIMIT),
            ..QueryOptions::default()
        };
        let report = engine
            .query(&domain, options)
            .map_err(|error| format!("{error:#}"))?;
        Ok(QueryOutcome {
            report,
            capabilities: engine.capabilities(),
            node_count: engine.node_count(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Write one query result into the results folder, in the CLI's JSON shape.
#[tauri::command]
async fn export_report(config: UiConfig, domain: String, format: String) -> Result<String, String> {
    let (index, sources, cfg) = index_and_sources(config)?;
    tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::open(&index, &sources).map_err(|error| format!("{error:#}"))?;
        let report = engine
            .query(
                &domain,
                QueryOptions {
                    metric: cfg.rank_metric,
                    ..QueryOptions::default()
                },
            )
            .map_err(|error| format!("{error:#}"))?;
        match format.as_str() {
            "csv" => {
                std::fs::create_dir_all(&cfg.paths.results_dir)
                    .map_err(|error| error.to_string())?;
                let name = web_radar_core::reverse::reverse_to_filename(&report.reverse_domain);
                let path = cfg.paths.results_dir.join(format!("{name}.csv"));
                std::fs::write(&path, report.to_target_result().to_csv())
                    .map_err(|error| error.to_string())?;
                Ok(path_string(&path))
            }
            _ => engine
                .write_result(&cfg.paths.results_dir, &report)
                .map(|path| path_string(&path))
                .map_err(|error| format!("{error:#}")),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The no-index path: stream every file for all configured targets.
#[tauri::command]
async fn start_full_scan(
    app: AppHandle,
    state: State<'_, AppState>,
    config: UiConfig,
) -> Result<Vec<String>, String> {
    if state.jobs.is_running() {
        return Err(
            "інша операція вже виконується — зупиніть її або дочекайтесь завершення".into(),
        );
    }
    let cfg = config.clone().into_engine_config()?;
    cfg.ensure_files().map_err(|error| format!("{error:#}"))?;
    if cfg.targets.is_empty() {
        return Err("додайте принаймні один домен".into());
    }

    let run_id = {
        let conn = state.db.lock().map_err(|_| "DB lock")?;
        let targets: Vec<String> = cfg.targets.iter().map(|t| t.domain.clone()).collect();
        db::start_run(
            &conn,
            "scan",
            cfg.rank_metric.as_str(),
            &targets,
            &path_string(&cfg.paths.results_dir),
        )
        .map_err(|e| e.to_string())?
    };
    let progress = state.jobs.begin(&app, run_id, "scan");
    let handle = state.jobs.clone_handle();

    let result = tauri::async_runtime::spawn_blocking(move || scan::run(&cfg, &progress))
        .await
        .map_err(|error| error.to_string())?;

    handle.finish();
    let error = result.as_ref().err().map(|error| format!("{error:#}"));
    {
        let conn = state.db.lock().map_err(|_| "DB lock")?;
        db::finish_run(&conn, run_id, error.as_deref()).map_err(|e| e.to_string())?;
    }
    result
        .map(|paths| paths.iter().map(|path| path_string(path)).collect())
        .map_err(|error| format!("{error:#}"))
}

// ---------------------------------------------------------------------------
// History and research tools
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_run_history(state: State<'_, AppState>) -> Result<Vec<RunRecord>, String> {
    let conn = state.db.lock().map_err(|_| "DB lock")?;
    db::history(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_seo_reports(state: State<'_, AppState>) -> Result<Vec<SeoReportRecord>, String> {
    let conn = state.db.lock().map_err(|_| "DB lock")?;
    db::seo_reports(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_manual_audit(
    request: manual_audit::ManualAuditRequest,
) -> Result<manual_audit::ManualAuditReport, String> {
    manual_audit::audit(request)
        .await
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn run_seo_discovery(
    state: State<'_, AppState>,
    request: seo::SeoDiscoveryRequest,
) -> Result<seo::SeoDiscoveryReport, String> {
    let report = seo::discover(request)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let json = serde_json::to_string(&report).map_err(|error| error.to_string())?;
    let conn = state.db.lock().map_err(|_| "DB lock")?;
    db::save_seo_report(&conn, &report.provider, &json).map_err(|error| error.to_string())?;
    Ok(report)
}

#[tauri::command]
async fn open_results_dir(config: UiConfig, app: AppHandle) -> Result<(), String> {
    let cfg = config.into_engine_config()?;
    std::fs::create_dir_all(&cfg.paths.results_dir).map_err(|error| error.to_string())?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(path_string(&cfg.paths.results_dir), None::<&str>)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(desktop)]
    {
        // Rule 28 of STAGE2_BRIEF: without this the installed copy never learns
        // that a new version exists. `process` is what restarts it afterwards.
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }

    builder
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let conn = db::open(&data_dir.join("web-radar.sqlite"))?;
            app.manage(AppState {
                db: Mutex::new(conn),
                config_path: Mutex::new(locate_config(app)),
                jobs: JobManager::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            product_metadata,
            load_default_config,
            save_config,
            index_status,
            build_index,
            drop_index_tier,
            cancel_job,
            query_domain,
            export_report,
            start_full_scan,
            open_results_dir,
            get_run_history,
            get_seo_reports,
            run_seo_discovery,
            run_manual_audit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Web Radar");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> UiConfig {
        UiConfig {
            vertices: "v.txt".into(),
            edges: "e.txt".into(),
            ranks: "r.txt".into(),
            results_dir: "results".into(),
            index_dir: "index".into(),
            rank_metric: "harmonic".into(),
            targets: vec!["example.com".into(), "  ".into()],
        }
    }

    #[test]
    fn ui_config_round_trips_through_the_engine_shape() {
        let cfg = sample_config().into_engine_config().expect("convert");
        assert_eq!(cfg.rank_metric, RankMetric::Harmonic);
        assert_eq!(cfg.targets.len(), 1, "blank rows must be dropped");

        let back = UiConfig::from_config(&cfg);
        assert_eq!(back.rank_metric, "harmonic");
        assert_eq!(back.targets, vec!["example.com".to_string()]);
    }

    #[test]
    fn rejects_an_unknown_metric_instead_of_defaulting() {
        let mut config = sample_config();
        config.rank_metric = "betweenness".into();
        assert!(config.into_engine_config().is_err());
    }

    #[test]
    fn generated_toml_parses_back_into_the_same_config() {
        let cfg = sample_config().into_engine_config().expect("convert");
        let text = toml_for(&cfg);
        let parsed: Config = toml::from_str(&text).expect("re-parse generated config");
        assert_eq!(parsed.rank_metric, RankMetric::Harmonic);
        assert_eq!(parsed.targets[0].domain, "example.com");
        assert_eq!(parsed.paths.index_dir, PathBuf::from("index"));
    }

    #[test]
    fn windows_paths_survive_the_toml_round_trip() {
        // Backslashes must be escaped or the next load reads a mangled path.
        let mut config = sample_config();
        let mut windows_path = PathBuf::from("D:");
        windows_path.push("graphs");
        windows_path.push("vertices.txt");
        config.vertices = windows_path.to_string_lossy().into_owned();

        let cfg = config.into_engine_config().expect("convert");
        let parsed: Config = toml::from_str(&toml_for(&cfg)).expect("re-parse");
        assert_eq!(parsed.paths.vertices, windows_path);
    }
}
