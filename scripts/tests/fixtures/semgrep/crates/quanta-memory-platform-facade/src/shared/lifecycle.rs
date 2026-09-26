use std::process::Command;
fn stale_filter() {
    // ruleid: qmp-isolated-child-requires-selection-guard
    let output = Command::new(std::env::current_exe().unwrap()).arg("missing").output();
}
fn stale_variable() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.arg("missing");
    // ruleid: qmp-isolated-child-requires-selection-guard
    let child = command.spawn();
}
fn guarded_chain() {
    let output = Command::new(std::env::current_exe().unwrap()).arg("one").require_one_test().output();
}
fn guarded_variable() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.arg("one");
    let selected = command.require_one_test();
    let child = selected.spawn();
}
fn ignored_guard_then_changed_filter() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.arg("one");
    let selected = command.require_one_test();
    command.arg("missing");
    // ruleid: qmp-isolated-child-requires-selection-guard
    let child = command.output();
}
fn unrelated_command() { Command::new("echo").arg("hello").status(); }
