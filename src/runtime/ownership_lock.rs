use super::{append_runtime_log, atomic_write_file, now_secs, RuntimeError, StatePaths};
use crate::runtime::supervisor::{load_supervisor_state, save_supervisor_state};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipState {
    NotRunning,
    Running { pid: u32 },
    Stale,
}

#[derive(Debug, Clone)]
pub struct StopResult {
    pub pid: u32,
    pub forced: bool,
}

pub fn supervisor_ownership_state(paths: &StatePaths) -> Result<OwnershipState, RuntimeError> {
    let state = load_supervisor_state(paths)?;
    if let Some(pid) = state.pid {
        if state.running && is_process_alive(pid) {
            return Ok(OwnershipState::Running { pid });
        }
    }

    if let Some(pid) = read_lock_pid(paths)? {
        if is_process_alive(pid) {
            return Ok(OwnershipState::Running { pid });
        }
        return Ok(OwnershipState::Stale);
    }

    if state.running || state.pid.is_some() {
        return Ok(OwnershipState::Stale);
    }

    Ok(OwnershipState::NotRunning)
}

pub fn cleanup_stale_supervisor(paths: &StatePaths) -> Result<(), RuntimeError> {
    let lock = paths.supervisor_lock_path();
    if lock.exists() {
        let _ = fs::remove_file(&lock);
    }
    let stop = paths.stop_signal_path();
    if stop.exists() {
        let _ = fs::remove_file(&stop);
    }
    let mut state = load_supervisor_state(paths)?;
    state.running = false;
    state.pid = None;
    state.stopped_at = Some(now_secs());
    save_supervisor_state(paths, &state)
}

pub fn reserve_start_lock(paths: &StatePaths) -> Result<(), RuntimeError> {
    let path = paths.supervisor_lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeError::CreateDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .and_then(|mut file| file.write_all(std::process::id().to_string().as_bytes()))
        .map_err(|source| RuntimeError::WriteLock {
            path: path.display().to_string(),
            source,
        })
}

pub fn write_supervisor_lock_pid(paths: &StatePaths, pid: u32) -> Result<(), RuntimeError> {
    let path = paths.supervisor_lock_path();
    atomic_write_file(&path, pid.to_string().as_bytes()).map_err(|source| RuntimeError::WriteLock {
        path: path.display().to_string(),
        source,
    })
}

pub fn clear_start_lock(paths: &StatePaths) {
    let _ = fs::remove_file(paths.supervisor_lock_path());
}

pub fn spawn_supervisor_process(state_root: &Path) -> Result<u32, RuntimeError> {
    let exe = std::env::current_exe().map_err(|e| RuntimeError::Spawn(e.to_string()))?;
    let child = std::process::Command::new(exe)
        .arg("__supervisor")
        .arg("--state-root")
        .arg(state_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| RuntimeError::Spawn(e.to_string()))?;
    Ok(child.id())
}

pub fn signal_stop(paths: &StatePaths) -> Result<(), RuntimeError> {
    let path = paths.stop_signal_path();
    fs::write(&path, b"stop").map_err(|source| RuntimeError::WriteState {
        path: path.display().to_string(),
        source,
    })
}

pub fn stop_active_supervisor(
    paths: &StatePaths,
    timeout: Duration,
) -> Result<StopResult, RuntimeError> {
    let pid = match supervisor_ownership_state(paths)? {
        OwnershipState::Running { pid } => pid,
        OwnershipState::Stale => {
            cleanup_stale_supervisor(paths)?;
            return Err(RuntimeError::NotRunning);
        }
        OwnershipState::NotRunning => return Err(RuntimeError::NotRunning),
    };

    signal_stop(paths)?;
    append_runtime_log(
        paths,
        "info",
        "supervisor.stop.requested",
        &format!("pid={pid}"),
    );

    let start = std::time::Instant::now();
    while is_process_alive(pid) && start.elapsed() < timeout {
        thread::sleep(Duration::from_millis(100));
    }

    let mut forced = false;
    if is_process_alive(pid) {
        send_signal(pid, "-TERM");
        let sigterm_start = std::time::Instant::now();
        while is_process_alive(pid) && sigterm_start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(100));
        }
    }

    if is_process_alive(pid) {
        forced = true;
        append_runtime_log(
            paths,
            "warn",
            "supervisor.stop.force_kill",
            &format!("pid={pid}"),
        );
        send_signal(pid, "-KILL");
        let sigkill_start = std::time::Instant::now();
        while is_process_alive(pid) && sigkill_start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(100));
        }
    }

    if is_process_alive(pid) {
        append_runtime_log(
            paths,
            "error",
            "supervisor.stop.failed",
            &format!("pid={pid} remained alive after TERM/KILL"),
        );
        return Err(RuntimeError::StopFailedAlive { pid });
    }

    cleanup_stale_supervisor(paths)?;
    Ok(StopResult { pid, forced })
}

fn read_lock_pid(paths: &StatePaths) -> Result<Option<u32>, RuntimeError> {
    let path = paths.supervisor_lock_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| RuntimeError::ReadLock {
        path: path.display().to_string(),
        source,
    })?;
    let parsed = raw.trim().parse::<u32>().ok();
    Ok(parsed)
}

pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        false
    }
}

fn send_signal(pid: u32, signal: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
    }
}
