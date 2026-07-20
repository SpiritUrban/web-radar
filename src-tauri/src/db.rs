use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub rank_metric: String,
    pub targets: Vec<String>,
    pub results_dir: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIndex {
    pub kind: String,
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub exists: bool,
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS runs (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           started_at TEXT NOT NULL,
           finished_at TEXT,
           status TEXT NOT NULL,
           rank_metric TEXT NOT NULL,
           targets_json TEXT NOT NULL,
           results_dir TEXT NOT NULL,
           error TEXT
         );
         CREATE TABLE IF NOT EXISTS file_index (
           kind TEXT PRIMARY KEY,
           path TEXT NOT NULL,
           size_bytes INTEGER NOT NULL,
           modified_at INTEGER,
           exists_flag INTEGER NOT NULL,
           indexed_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_runs_started_at ON runs(started_at DESC);
         CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
         CREATE TABLE IF NOT EXISTS seo_reports (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           generated_at TEXT NOT NULL,
           provider TEXT NOT NULL,
           report_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_seo_reports_generated_at ON seo_reports(generated_at DESC);"
    )?;
    Ok(conn)
}

pub fn start_run(conn: &Connection, metric: &str, targets: &[String], results: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO runs(started_at,status,rank_metric,targets_json,results_dir) VALUES (?1,'running',?2,?3,?4)",
        params![chrono::Utc::now().to_rfc3339(), metric, serde_json::to_string(targets)?, results],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_run(conn: &Connection, id: i64, error: Option<&str>) -> Result<()> {
    let status = if error.is_some() { "failed" } else { "completed" };
    conn.execute(
        "UPDATE runs SET finished_at=?1,status=?2,error=?3 WHERE id=?4",
        params![chrono::Utc::now().to_rfc3339(), status, error, id],
    )?;
    Ok(())
}

pub fn history(conn: &Connection) -> Result<Vec<RunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id,started_at,finished_at,status,rank_metric,targets_json,results_dir,error FROM runs ORDER BY id DESC LIMIT 100"
    )?;
    let rows = stmt.query_map([], |row| {
        let json: String = row.get(5)?;
        Ok(RunRecord {
            id: row.get(0)?, started_at: row.get(1)?, finished_at: row.get(2)?,
            status: row.get(3)?, rank_metric: row.get(4)?,
            targets: serde_json::from_str(&json).unwrap_or_default(),
            results_dir: row.get(6)?, error: row.get(7)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn replace_file_index(conn: &mut Connection, files: &[FileIndex]) -> Result<()> {
    let tx = conn.transaction()?;
    for file in files {
        tx.execute(
            "INSERT INTO file_index(kind,path,size_bytes,modified_at,exists_flag,indexed_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(kind) DO UPDATE SET path=excluded.path,size_bytes=excluded.size_bytes,
             modified_at=excluded.modified_at,exists_flag=excluded.exists_flag,indexed_at=excluded.indexed_at",
            params![file.kind, file.path, file.size_bytes, file.modified_at, file.exists, chrono::Utc::now().to_rfc3339()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn save_seo_report(conn: &Connection, provider: &str, report_json: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO seo_reports(generated_at,provider,report_json) VALUES (?1,?2,?3)",
        params![chrono::Utc::now().to_rfc3339(), provider, report_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn file_index(conn: &Connection) -> Result<Vec<FileIndex>> {
    let mut stmt = conn.prepare("SELECT kind,path,size_bytes,modified_at,exists_flag FROM file_index ORDER BY kind")?;
    let rows = stmt.query_map([], |row| Ok(FileIndex {
        kind: row.get(0)?, path: row.get(1)?, size_bytes: row.get::<_, i64>(2)? as u64,
        modified_at: row.get(3)?, exists: row.get(4)?,
    }))?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema_and_reads_file_index() {
        let conn = Connection::open_in_memory().expect("open SQLite");
        conn.execute_batch(
            "CREATE TABLE file_index (
                kind TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                modified_at INTEGER,
                exists_flag INTEGER NOT NULL,
                indexed_at TEXT NOT NULL
            );"
        ).expect("create file index schema");
        assert!(file_index(&conn).expect("read file index").is_empty());
    }
}