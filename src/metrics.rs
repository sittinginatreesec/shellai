use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};

use crate::config::Config;

pub fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}-{}", std::process::id(), unix_time_ms())
}

pub fn server_path() -> Result<PathBuf> {
    Ok(metrics_dir()?.join("server.jsonl"))
}

pub fn client_path() -> Result<PathBuf> {
    Ok(metrics_dir()?.join("client.jsonl"))
}

pub fn append_server<T: Serialize>(value: &T) -> Result<()> {
    append_jsonl(&server_path()?, value)
}

pub fn append_client<T: Serialize>(value: &T) -> Result<()> {
    append_jsonl(&client_path()?, value)
}

fn metrics_dir() -> Result<PathBuf> {
    let path = Config::cache_dir()?.join("metrics");
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create metrics directory {}", path.display()))?;
    Ok(path)
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open metrics file {}", path.display()))?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub struct MemoryMonitor {
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<u64>,
    before: u64,
}

impl MemoryMonitor {
    pub fn start() -> Self {
        let before = current_rss_bytes();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut peak = before;
            while !thread_stop.load(Ordering::Relaxed) {
                peak = peak.max(current_rss_bytes());
                thread::sleep(Duration::from_millis(10));
            }
            peak.max(current_rss_bytes())
        });
        Self {
            stop,
            handle,
            before,
        }
    }

    pub fn finish(self) -> MemoryUsage {
        self.stop.store(true, Ordering::Relaxed);
        let after = current_rss_bytes();
        let peak = self
            .handle
            .join()
            .unwrap_or(self.before.max(after))
            .max(after);
        MemoryUsage {
            before: self.before,
            after,
            peak,
        }
    }
}

pub struct MemoryUsage {
    pub before: u64,
    pub after: u64,
    pub peak: u64,
}

fn current_rss_bytes() -> u64 {
    let Ok(pid) = get_current_pid() else {
        return 0;
    };
    let mut system = System::new();
    let pids = [pid];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );
    system.process(pid).map_or(0, |process| process.memory())
}
