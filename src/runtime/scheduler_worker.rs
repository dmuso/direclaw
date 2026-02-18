use crate::config::Settings;
use crate::orchestration::scheduler::SchedulerWorker;
use crate::runtime::append_runtime_log;
use crate::runtime::StatePaths;

pub fn tick_scheduler_worker(
    state_root: &std::path::Path,
    settings: &Settings,
) -> Result<(), String> {
    let mut dispatched_total = 0usize;

    for orchestrator_id in settings.orchestrators.keys() {
        let runtime_root = settings
            .resolve_orchestrator_runtime_root(orchestrator_id)
            .map_err(|err| err.to_string())?;
        let mut worker = SchedulerWorker::new(&runtime_root);
        let runs = worker.tick(now_secs())?;
        dispatched_total = dispatched_total.saturating_add(runs.len());
    }

    append_runtime_log(
        &StatePaths::new(state_root),
        "info",
        "scheduler.tick",
        &format!("dispatched={dispatched_total}"),
    );

    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
