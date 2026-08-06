//! Progress reporting and cooperative cancellation for long file passes.
//!
//! Index builds read tens of gigabytes. Two things make that bearable in a UI:
//! an honest ETA computed from *measured* throughput, and a cancel button that
//! actually stops the work instead of just hiding the window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Returned when the caller asked to stop mid-pass.
#[derive(Debug, Clone, Copy)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("операцію скасовано")
    }
}

impl std::error::Error for Cancelled {}

/// One progress tick, shaped for direct serialization to the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressUpdate {
    /// Machine-readable stage key (`vertices_index`, `inbound_partition`, …).
    pub stage: String,
    /// Human-readable detail for the current stage.
    pub detail: String,
    /// Progress inside the current stage.
    pub stage_done: u64,
    pub stage_total: u64,
    /// Progress across the whole operation, `0.0..=1.0`.
    pub overall: f64,
    /// Measured bytes per second for the current stage (0 when unknown).
    pub bytes_per_sec: u64,
    /// Estimate for the whole operation, seconds (0 when unknown).
    pub eta_secs: u64,
    pub elapsed_secs: u64,
}

struct Stage {
    key: String,
    detail: String,
    base: f64,
    weight: f64,
    total: u64,
    done: u64,
    started: Instant,
    /// Bytes already processed when the stage began (for multi-file stages).
    counts_bytes: bool,
}

struct Inner {
    stage: Option<Stage>,
    last_emit: Instant,
}

type Sink = dyn Fn(ProgressUpdate) + Send + Sync;

/// Progress sink + cancellation flag handed down through the pipeline.
#[derive(Clone)]
pub struct Progress {
    sink: Option<Arc<Sink>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    inner: Arc<Mutex<Inner>>,
}

impl Default for Progress {
    fn default() -> Self {
        Self::silent()
    }
}

impl Progress {
    /// A progress handle that reports nothing and is never cancelled.
    pub fn silent() -> Self {
        Self {
            sink: None,
            cancel: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            inner: Arc::new(Mutex::new(Inner {
                stage: None,
                last_emit: Instant::now() - Duration::from_secs(1),
            })),
        }
    }

    pub fn new<F>(cancel: Arc<AtomicBool>, sink: F) -> Self
    where
        F: Fn(ProgressUpdate) + Send + Sync + 'static,
    {
        Self {
            sink: Some(Arc::new(sink)),
            cancel,
            ..Self::silent()
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once the caller flipped the cancel flag.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Open a stage occupying `[base, base + weight]` of the overall bar.
    pub fn stage(&self, key: &str, detail: impl Into<String>, base: f64, weight: f64, total: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.stage = Some(Stage {
            key: key.to_string(),
            detail: detail.into(),
            base,
            weight,
            total,
            done: 0,
            started: Instant::now(),
            counts_bytes: true,
        });
        drop(inner);
        self.emit(true);
    }

    /// Replace the human-readable detail of the current stage.
    pub fn detail(&self, detail: impl Into<String>) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stage) = inner.stage.as_mut() {
                stage.detail = detail.into();
            }
        }
        self.emit(true);
    }

    /// Absolute position inside the current stage.
    pub fn set(&self, done: u64) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stage) = inner.stage.as_mut() {
                stage.done = done;
            }
        }
        self.emit(false);
    }

    /// Relative movement inside the current stage.
    pub fn advance(&self, delta: u64) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stage) = inner.stage.as_mut() {
                stage.done += delta;
            }
        }
        self.emit(false);
    }

    /// Close the current stage, snapping the bar to its end.
    pub fn finish_stage(&self) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stage) = inner.stage.as_mut() {
                stage.done = stage.total;
                stage.counts_bytes = false;
            }
        }
        self.emit(true);
    }

    fn emit(&self, force: bool) {
        let Some(sink) = self.sink.as_ref() else {
            return;
        };
        let update = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            // Ten updates a second is plenty for a human and cheap for the IPC bridge.
            if !force && now.duration_since(inner.last_emit) < Duration::from_millis(100) {
                return;
            }
            inner.last_emit = now;

            let Some(stage) = inner.stage.as_mut() else {
                return;
            };
            let ratio = if stage.total > 0 {
                (stage.done as f64 / stage.total as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let overall = (stage.base + stage.weight * ratio).clamp(0.0, 1.0);
            let stage_secs = now.duration_since(stage.started).as_secs_f64().max(0.001);
            let bytes_per_sec = if stage.counts_bytes {
                (stage.done as f64 / stage_secs) as u64
            } else {
                0
            };
            let elapsed = now.duration_since(self.started).as_secs_f64();
            // ETA from overall progress, not from the current stage: stages have
            // wildly different throughput and a per-stage ETA jumps at every seam.
            let eta_secs = if overall > 0.01 && overall < 1.0 {
                (elapsed / overall - elapsed).max(0.0) as u64
            } else {
                0
            };
            ProgressUpdate {
                stage: stage.key.clone(),
                detail: stage.detail.clone(),
                stage_done: stage.done,
                stage_total: stage.total,
                overall,
                bytes_per_sec,
                eta_secs,
                elapsed_secs: elapsed as u64,
            }
        };
        sink(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_monotonic_overall_progress_and_respects_cancel() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Progress::new(Arc::clone(&cancel), move |update| {
            sink.lock().unwrap().push(update.overall);
        });

        progress.stage("a", "first", 0.0, 0.5, 100);
        progress.finish_stage();
        progress.stage("b", "second", 0.5, 0.5, 100);
        progress.finish_stage();

        let seen = seen.lock().unwrap().clone();
        assert!(!seen.is_empty(), "sink must receive updates");
        assert!(
            seen.windows(2).all(|w| w[1] >= w[0]),
            "overall must not go backwards: {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some(1.0));

        assert!(progress.check().is_ok());
        cancel.store(true, Ordering::Relaxed);
        assert!(progress.check().is_err());
    }
}
