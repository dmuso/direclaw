use crate::config::Settings;
use std::time::Duration;

pub fn configured_heartbeat_interval(settings: &Settings) -> Option<Duration> {
    let seconds = settings.monitoring.heartbeat_interval.unwrap_or(0);
    if seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(seconds))
    }
}

pub fn tick_heartbeat_worker() -> Result<(), String> {
    Ok(())
}
