//! Local SQLite history: what was run, when, and how it ended.

use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: i64,
    /// `index` or `scan`.
    pub kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub rank_metric: String,
    pub targets: Vec<String>,
    pub output_dir: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoReportRecord {
    pub id: i64,
    pub generated_at: String,
    pub provider: String,
    pub evidence_count: i64,
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
         CREATE INDEX IF NOT EXISTS idx_runs_started_at ON runs(started_at DESC);
         CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
         CREATE TABLE IF NOT EXISTS seo_reports (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           generated_at TEXT NOT NULL,
           provider TEXT NOT NULL,
           report_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_seo_reports_generated_at ON seo_reports(generated_at DESC);",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

/// Additive migrations. `ALTER TABLE ADD COLUMN` fails when the column is
/// already there, which is the normal case on every launch after the first.
fn migrate(conn: &Connection) -> Result<()> {
    let has_kind = conn
        .prepare("SELECT 1 FROM pragma_table_info('runs') WHERE name = 'kind'")?
        .exists([])?;
    if !has_kind {
        conn.execute_batch("ALTER TABLE runs ADD COLUMN kind TEXT NOT NULL DEFAULT 'scan'")?;
    }
    // The pre-0.3 file inventory is superseded by the index status view.
    conn.execute_batch("DROP TABLE IF EXISTS file_index")?;
    Ok(())
}

pub fn start_run(
    conn: &Connection,
    kind: &str,
    metric: &str,
    targets: &[String],
    output_dir: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO runs(kind,started_at,status,rank_metric,targets_json,results_dir)
         VALUES (?1,?2,'running',?3,?4,?5)",
        params![
            kind,
            chrono::Utc::now().to_rfc3339(),
            metric,
            serde_json::to_string(targets)?,
            output_dir
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_run(conn: &Connection, id: i64, error: Option<&str>) -> Result<()> {
    let status = match error {
        Some(message) if message.contains("скасовано") => "cancelled",
        Some(_) => "failed",
        None => "completed",
    };
    conn.execute(
        "UPDATE runs SET finished_at=?1,status=?2,error=?3 WHERE id=?4",
        params![chrono::Utc::now().to_rfc3339(), status, error, id],
    )?;
    Ok(())
}

pub fn history(conn: &Connection) -> Result<Vec<RunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id,kind,started_at,finished_at,status,rank_metric,targets_json,results_dir,error
         FROM runs ORDER BY id DESC LIMIT 100",
    )?;
    let rows = stmt.query_map([], |row| {
        let targets_json: String = row.get(6)?;
        Ok(RunRecord {
            id: row.get(0)?,
            kind: row.get(1)?,
            started_at: row.get(2)?,
            finished_at: row.get(3)?,
            status: row.get(4)?,
            rank_metric: row.get(5)?,
            targets: serde_json::from_str(&targets_json).unwrap_or_default(),
            output_dir: row.get(7)?,
            error: row.get(8)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn save_seo_report(conn: &Connection, provider: &str, report_json: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO seo_reports(generated_at,provider,report_json) VALUES (?1,?2,?3)",
        params![chrono::Utc::now().to_rfc3339(), provider, report_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn seo_reports(conn: &Connection) -> Result<Vec<SeoReportRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id,generated_at,provider,report_json FROM seo_reports ORDER BY id DESC LIMIT 50",
    )?;
    let rows = stmt.query_map([], |row| {
        let json: String = row.get(3)?;
        let evidence_count = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|value| {
                value
                    .get("evidence")
                    .and_then(|e| e.as_array())
                    .map(|a| a.len())
            })
            .unwrap_or(0) as i64;
        Ok(SeoReportRecord {
            id: row.get(0)?,
            generated_at: row.get(1)?,
            provider: row.get(2)?,
            evidence_count,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_run_from_start_to_finish() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open(&dir.path().join("web-radar.sqlite")).expect("open");

        let id = start_run(
            &conn,
            "scan",
            "pagerank",
            &["example.com".into()],
            "results",
        )
        .expect("start");
        let running = history(&conn).expect("history");
        assert_eq!(running[0].status, "running");
        assert_eq!(running[0].kind, "scan");
        assert_eq!(running[0].targets, vec!["example.com".to_string()]);

        finish_run(&conn, id, None).expect("finish");
        assert_eq!(history(&conn).expect("history")[0].status, "completed");
    }

    #[test]
    fn a_cancelled_run_is_not_recorded_as_a_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open(&dir.path().join("db.sqlite")).expect("open");
        let id = start_run(&conn, "index", "pagerank", &[], "index").expect("start");
        finish_run(&conn, id, Some("операцію скасовано")).expect("finish");
        assert_eq!(history(&conn).expect("history")[0].status, "cancelled");
    }

    #[test]
    fn upgrades_a_pre_0_3_database_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("old.sqlite");
        // Exactly the 0.2 schema, including the table that 0.3 drops.
        let old = Connection::open(&path).expect("create");
        old.execute_batch(
            "CREATE TABLE runs (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               started_at TEXT NOT NULL, finished_at TEXT, status TEXT NOT NULL,
               rank_metric TEXT NOT NULL, targets_json TEXT NOT NULL,
               results_dir TEXT NOT NULL, error TEXT);
             INSERT INTO runs(started_at,status,rank_metric,targets_json,results_dir)
               VALUES ('2026-01-01T00:00:00Z','completed','pagerank','[\"old.com\"]','results');
             CREATE TABLE file_index (kind TEXT PRIMARY KEY, path TEXT NOT NULL);",
        )
        .expect("seed old schema");
        drop(old);

        let conn = open(&path).expect("open upgrades");
        let rows = history(&conn).expect("history");
        assert_eq!(rows.len(), 1, "existing history must survive the upgrade");
        assert_eq!(rows[0].kind, "scan", "old rows get the default kind");
        assert_eq!(rows[0].targets, vec!["old.com".to_string()]);
    }

    #[test]
    fn seo_reports_are_summarised_without_re_reading_the_whole_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open(&dir.path().join("db.sqlite")).expect("open");
        save_seo_report(&conn, "brave", r#"{"evidence":[{"url":"a"},{"url":"b"}]}"#).expect("save");
        let reports = seo_reports(&conn).expect("list");
        assert_eq!(reports[0].provider, "brave");
        assert_eq!(reports[0].evidence_count, 2);
    }
}
