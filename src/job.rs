use anyhow::{Result, bail};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub phase: &'static str,
    pub file: String,
    pub done: u64,
    pub total: u64,
}
#[derive(Clone, Default)]
pub struct Control {
    cancelled: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}
impl Control {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!("Task cancelled");
        }
        Ok(())
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    pub fn set(&self, phase: &'static str, file: impl Into<String>, done: u64, total: u64) {
        *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = Progress {
            phase,
            file: file.into(),
            done,
            total,
        };
    }
    pub fn progress(&self) -> Progress {
        self.progress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}
