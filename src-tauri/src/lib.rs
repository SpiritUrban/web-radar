#[path = "../../src/config.rs"]
mod config;
#[path = "../../src/processor.rs"]
mod processor;
#[path = "../../src/reverse.rs"]
mod reverse;
mod db;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use config::{Config, PathsConfig, RankMetric, Target};
use db::{FileIndex, RunRecord};

struct AppState {
    db: Mutex<rusqlite::Connection>,
    config_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiConfig {
    vertices: String,
    edges: String,
    ranks: String,
    results_dir: String,
    rank_metric: String,
    targets: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    run_id: i64,
    phase: String,
    message: String,
    progress: f64,
}

impl UiConfig {
    fn from_config(cfg: Config) -> Self {
        Self {
            vertices: cfg.paths.vertices.to_string_lossy().into_owned(),
            edges: cfg.paths.edges.to_string_lossy().into_owned(),
            ranks: cfg.paths.ranks.to_string_lossy().into_owned(),
            results_dir: cfg.paths.results_dir.to_string_lossy().into_owned(),
            rank_metric: cfg.rank_metric.as_str().to_string(),
            targets: cfg.targets.into_iter().map(|t| t.domain).collect(),
        }
    }

    fn into_engine(self) -> Result<Config, String> {
        let rank_metric = match self.rank_metric.as_str() {
            "pagerank" => RankMetric::Pagerank,
            "harmonic" => RankMetric::Harmonic,
            other => return Err(format!("Невідома метрика: {other}")),
        };
        if self.targets.iter().all(|x| x.trim().is_empty()) {
            return Err("Додайте принаймні один домен".into());
        }
        Ok(Config {
            paths: PathsConfig {
                vertices: PathBuf::from(self.vertices),
                edges: PathBuf::from(self.edges),
                ranks: PathBuf::from(self.ranks),
                results_dir: PathBuf::from(self.results_dir),
            },
            rank_metric,
            targets: self.targets.into_iter().filter(|x| !x.trim().is_empty()).map(|domain| Target { domain }).collect(),
            config_dir: PathBuf::new(),
        })
    }
}

fn locate_config(app: &tauri::App) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut candidates = vec![
        cwd.join("config.toml"),
        cwd.parent().unwrap_or(&cwd).join("config.toml"),
    ];

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

#[tauri::command]
fn load_default_config(state: State<'_, AppState>) -> Result<UiConfig, String> {
    Config::load(&state.config_path).map(UiConfig::from_config).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn get_run_history(state: State<'_, AppState>) -> Result<Vec<RunRecord>, String> {
    let conn = state.db.lock().map_err(|_| "DB lock")?;
    db::history(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_file_index(state: State<'_, AppState>) -> Result<Vec<FileIndex>, String> {
    let conn = state.db.lock().map_err(|_| "DB lock")?;
    db::file_index(&conn).map_err(|e| e.to_string())
}

fn inspect_files(cfg: &Config) -> Vec<FileIndex> {
    [
        ("vertices", cfg.paths.vertices.as_path()),
        ("edges", cfg.paths.edges.as_path()),
        ("ranks", cfg.paths.ranks.as_path()),
    ].par_iter().map(|(kind, path)| {
        let metadata = std::fs::metadata(path).ok();
        FileIndex {
            kind: (*kind).to_string(),
            path: path.to_string_lossy().into_owned(),
            size_bytes: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified_at: metadata.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64),
            exists: path.is_file(),
        }
    }).collect()
}

fn emit(app: &AppHandle, run_id: i64, phase: &str, message: &str, progress: f64) {
    let _ = app.emit("run-progress", ProgressEvent {
        run_id, phase: phase.into(), message: message.into(), progress,
    });
}

#[tauri::command]
async fn start_analysis(app: AppHandle, state: State<'_, AppState>, config: UiConfig) -> Result<(), String> {
    let metric = config.rank_metric.clone();
    let targets = config.targets.clone();
    let results = config.results_dir.clone();
    let cfg = config.into_engine()?;
    let files = inspect_files(&cfg);
    if let Some(missing) = files.iter().find(|f| !f.exists) {
        return Err(format!("Файл {} не знайдено:\n{}", missing.kind, missing.path));
    }
    let run_id = {
        let mut conn = state.db.lock().map_err(|_| "DB lock")?;
        db::replace_file_index(&mut conn, &files).map_err(|e| e.to_string())?;
        db::start_run(&conn, &metric, &targets, &results).map_err(|e| e.to_string())?
    };
    emit(&app, run_id, "index", "Файли перевірено та проіндексовано", 0.08);
    emit(&app, run_id, "processing", "Сканування графа Common Crawl…", 0.15);

    let result = tokio::task::spawn_blocking(move || processor::run(&cfg))
        .await.map_err(|e| e.to_string())?;
    let error = result.as_ref().err().map(|e| format!("{e:#}"));
    {
        let conn = state.db.lock().map_err(|_| "DB lock")?;
        db::finish_run(&conn, run_id, error.as_deref()).map_err(|e| e.to_string())?;
    }
    match error {
        Some(error) => {
            emit(&app, run_id, "failed", "Аналіз завершився з помилкою", 1.0);
            Err(error)
        }
        None => {
            emit(&app, run_id, "complete", "Результати готові", 1.0);
            Ok(())
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let conn = db::open(&data_dir.join("web-radar.sqlite"))?;
            app.manage(AppState { db: Mutex::new(conn), config_path: locate_config(app) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_default_config, get_run_history, get_file_index, start_analysis
        ])
        .run(tauri::generate_context!())
        .expect("error while running Web Radar");
}


