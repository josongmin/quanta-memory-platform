// ruleid: qmp-pure-layer-no-adapter-import
use crate::adapters::Allocator;
fn forbidden() {
    // ruleid: qmp-pure-layer-no-fs-env
    let _ = std::env::var("QMP");
    // ruleid: qmp-pure-layer-no-wall-clock
    let _ = std::time::Instant::now();
}
fn pure(clock: &dyn Clock) { let _ = clock.now(); }
