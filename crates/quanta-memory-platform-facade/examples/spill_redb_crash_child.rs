//! Crash-child helper for the real `SIGKILL` / reopen qualification rail.
//!
//! This is test tooling, not a consumer demo: `tests/spill_redb_kill_reopen.rs`
//! spawns this binary, waits for its `READY` handshake, kills it with
//! [`std::process::Child::kill`] (`SIGKILL` on Unix — no destructors, no
//! flush, no cleanup), then reopens the same database file in the parent and
//! asserts exact recovery. It uses only the public durable spill surface, so
//! the rail also proves a real host can drive restart recovery without
//! crate-internal helpers.
//!
//! Protocol (stdout, line buffered):
//!
//! - `fill <count>`: commit `<count>` records, `prepare_restore` the oldest
//!   (persisting an active claim), print `BACKEND <u128>` then `READY`, sleep.
//! - `stream`: commit record zero, print `READY`, then continue committing in a
//!   tight loop until killed.
//! - `hole`: commit two records, reserve one id WITHOUT committing it, print
//!   `HOLE <id>` then `READY`, sleep.
//!
//! Any backend failure before `READY` exits non-zero WITHOUT printing `READY`,
//! so the parent fails closed instead of qualifying an empty database.
//!
//! Run only via the integration test:
//!
//! ```bash
//! cargo test --locked -p quanta-memory-platform-facade --features spill-redb \
//!     --test spill_redb_kill_reopen
//! ```

use std::io::Write;
use std::time::Duration;

use quanta_memory_platform_facade::spill::backings::redb::RedbSpillBacking;
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: spill_redb_crash_child <db-path> <fill <count>|stream|hole>");
        std::process::exit(2);
    }
    let backing = RedbSpillBacking::create(&args[1]).expect("child open");
    let mut stdout = std::io::stdout().lock();
    match args[2].as_str() {
        "fill" => {
            let count: u64 = args.get(3).expect("fill count").parse().expect("count");
            for index in 0..count {
                commit_one(&backing, index);
            }
            let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
            writeln!(stdout, "BACKEND {}", candidate.claim.backend()).expect("BACKEND line");
            writeln!(stdout, "READY").expect("READY line");
            stdout.flush().expect("flush");
            std::thread::sleep(Duration::from_secs(120));
        }
        "stream" => {
            commit_one(&backing, 0);
            writeln!(stdout, "READY").expect("READY line");
            stdout.flush().expect("flush");
            let mut index = 1u64;
            loop {
                commit_one(&backing, index);
                index += 1;
            }
        }
        "hole" => {
            commit_one(&backing, 0);
            commit_one(&backing, 1);
            let hole = backing.reserve_record_id().expect("hole reserve");
            writeln!(stdout, "HOLE {}", hole.get()).expect("HOLE line");
            writeln!(stdout, "READY").expect("READY line");
            stdout.flush().expect("flush");
            std::thread::sleep(Duration::from_secs(120));
        }
        mode => {
            eprintln!("unknown mode: {mode}");
            std::process::exit(2);
        }
    }
}
