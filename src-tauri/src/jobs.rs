//! One long-running job at a time, with progress events and a cancel button.
//!
//! Index builds and full scans read tens of gigabytes; both have to be
//! stoppable, and both report through the same `job-progress` event so the UI
//! needs one listener rather than one per operation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use web_radar_core::progress::{Progress, ProgressUpdate};

/// Payload of the `job-progress` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub run_id: i64,
    /// `index` or `scan` — the UI labels the bar accordingly.
    pub kind: String,
    #[serde(flatten)]
    pub update: ProgressUpdate,
}

#[derive(Clone, Default)]
pub struct JobManager {
    cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    running: Arc<AtomicBool>,
}

/// Handle used to mark the job finished once the worker returns.
pub struct JobHandle {
    running: Arc<AtomicBool>,
    cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl JobHandle {
    pub fn finish(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.cancel.lock() {
            *slot = None;
        }
    }
}

impl JobManager {
    /// Start a job and return the [`Progress`] its worker should report through.
    pub fn begin(&self, app: &AppHandle, run_id: i64, kind: &str) -> Progress {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut slot) = self.cancel.lock() {
            *slot = Some(Arc::clone(&flag));
        }
        self.running.store(true, Ordering::Relaxed);

        let app = app.clone();
        let kind = kind.to_string();
        Progress::new(flag, move |update| {
            let _ = app.emit(
                "job-progress",
                JobProgress {
                    run_id,
                    kind: kind.clone(),
                    update,
                },
            );
        })
    }

    /// Ask the running job to stop at its next checkpoint.
    pub fn cancel(&self) {
        if let Ok(slot) = self.cancel.lock() {
            if let Some(flag) = slot.as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn clone_handle(&self) -> JobHandle {
        JobHandle {
            running: Arc::clone(&self.running),
            cancel: Arc::clone(&self.cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_reaches_the_running_job_and_clears_on_finish() {
        let manager = JobManager::default();
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut slot) = manager.cancel.lock() {
            *slot = Some(Arc::clone(&flag));
        }
        manager.running.store(true, Ordering::Relaxed);

        assert!(manager.is_running());
        manager.cancel();
        assert!(
            flag.load(Ordering::Relaxed),
            "cancel must reach the worker's flag"
        );

        manager.clone_handle().finish();
        assert!(!manager.is_running());
        // A later cancel must not resurrect a finished job.
        manager.cancel();
        assert!(manager.cancel.lock().unwrap().is_none());
    }
}
