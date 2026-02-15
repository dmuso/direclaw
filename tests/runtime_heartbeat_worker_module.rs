use direclaw::runtime::heartbeat_worker::tick_heartbeat_worker;

#[test]
fn runtime_heartbeat_worker_module_exposes_tick() {
    tick_heartbeat_worker().expect("heartbeat tick should be a no-op success");
}
