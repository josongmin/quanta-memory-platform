//! Finalize-kill child helper for the DURABILITY-001 L1 rail.
//!
//! This is test tooling, not a consumer demo:
//! `tests/spill_redb_finalize_kill.rs` spawns this binary, waits for its
//! `ARMED` handshake, kills it with [`std::process::Child::kill`] (`SIGKILL`
//! on Unix — no destructors, no flush, no cleanup), then reopens the same
//! database file in the parent and asserts the point-specific oracle. It uses
//! only the public durable spill surface plus the single test-only hook it
//! installs, so the rail proves a real host can drive restart recovery without
//! crate-internal helpers.
//!
//! Protocol (stdout, flushed before blocking):
//!
//! - `<db-path> before`: commit two records, `prepare_restore` +
//!   `commit_restore` the oldest (victim), print `BACKEND <u128>`, arm the
//!   `FinalizeBeforeCommit` hook, then `finalize_restore`. The hook emits
//!   `ARMED FinalizeBeforeCommit <sequence> <generation>` from inside the
//!   finalize transaction — commit not yet called — and blocks forever.
//! - `<db-path> after`: same flow with the `FinalizeAfterCommit` hook, which
//!   emits `ARMED FinalizeAfterCommit <sequence> <generation>` immediately
//!   after `txn.commit()` returns `Ok(())` and blocks forever.
//!
//! Any backend failure before `ARMED` exits non-zero WITHOUT printing `ARMED`,
//! and a `finalize_restore` that returns without firing the hook exits `3`, so
//! the parent fails closed instead of qualifying a missed kill point.
//!
//! Run only via the integration test:
//!
//! ```bash
//! cargo test --locked -p quanta-memory-platform-facade \
//!     --features spill-redb,durability-test-hooks \
//!     --test spill_redb_finalize_kill
//! ```

use std::io::Write;

use quanta_memory_platform_facade::spill::backings::redb::{
    finalize_kill_hooks::{install, FinalizeKillPoint},
    RedbSpillBacking,
};
use quanta_memory_platform_facade::spill::service::{
    DurableRestoreTarget, ReconciledDurableSpillTarget,
};

/// 32-byte self-describing payload: little-endian record index plus filler.
fn payload(index: u64) -> Vec<u8> {
    let mut bytes = vec![0xA5u8; 32];
    bytes[..8].copy_from_slice(&index.to_le_bytes());
    bytes
}

fn commit_one(backing: &RedbSpillBacking, index: u64) {
    let id = backing.reserve_record_id().expect("child reserve");
    match backing.commit_spill(id, &payload(index)) {
        Ok(_) => {}
        Err(error) => panic!("child commit {index} failed: {error:?}"),
    }
}

/// The armed hook: prove the exact kill point over the control pipe, then
/// block forever. Parking (never unparked) means this function never returns,
/// so the child can only ever end by the parent's `SIGKILL`.
fn armed_hook(point: FinalizeKillPoint, sequence: u64, generation: u64) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "ARMED {} {sequence} {generation}", point.token()).expect("ARMED line");
    stdout.flush().expect("ARMED flush");
    drop(stdout);
    loop {
        std::thread::park();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: spill_redb_finalize_kill_child <db-path> <before|after>");
        std::process::exit(2);
    }
    let point = match args[2].as_str() {
        "before" => FinalizeKillPoint::BeforeCommit,
        "after" => FinalizeKillPoint::AfterCommit,
        mode => {
            eprintln!("unknown mode: {mode}");
            std::process::exit(2);
        }
    };
    let backing = RedbSpillBacking::create(&args[1]).expect("child open");
    // Two records: the victim (oldest, sequence 0) plus one unrelated record
    // whose claim and proof must survive the kill untouched.
    commit_one(&backing, 0);
    commit_one(&backing, 1);
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(candidate.sequence, 0, "the victim is the oldest record");
    backing.commit_restore(&candidate.claim).expect("commit");
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "BACKEND {}", candidate.claim.backend()).expect("BACKEND line");
    stdout.flush().expect("BACKEND flush");
    drop(stdout);
    install(point, armed_hook);
    backing.finalize_restore(&candidate.claim).expect("finalize");
    // Unreachable when the armed hook fired: falling through would let the
    // parent mistake a missed hook for a kill boundary.
    eprintln!("finalize returned without firing the armed hook");
    std::process::exit(3);
}
