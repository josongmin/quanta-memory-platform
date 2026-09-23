//! Crash-child helper for the real `SIGKILL` / reopen qualification rail.
//!
//! This is test tooling, not a consumer demo: `tests/spill_redb_kill_reopen.rs`
//! spawns this binary, waits for its `READY` handshake, kills it with
//! [`std::process::Child::kill`] (`SIGKILL` on Unix — no destructors, no
//! flush, no cleanup), then reopens the same database file in the parent and
//! asserts exact recovery. Backing-only modes use the public durable spill
//! surface; `store-spilled` also uses the public platform/store builders.
//!
//! Protocol (stdout, line buffered):
//!
//! - `fill <count>`: commit `<count>` records, `prepare_restore` the oldest
//!   (persisting an active claim), print `BACKEND <u128>` then `READY`, sleep.
//! - `stream`: commit record zero, print `READY`, then continue committing in a
//!   tight loop until killed.
//! - `hole`: commit two records, reserve one id WITHOUT committing it, print
//!   `HOLE <id>` then `READY`, sleep.
//! - `store-spilled` (`revocable_store`): spill one charged store entry, prove
//!   the child ledger drained, print `CHARGE 0` then `READY`, park.
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

#[cfg(feature = "revocable_store")]
use quanta_memory_platform_core::{MemoryClass, MemoryPreset, StoreBindingHandle};
use quanta_memory_platform_facade::spill::backings::redb::RedbSpillBacking;
use quanta_memory_platform_facade::spill::service::{
    DurableRestoreTarget, ReconciledDurableSpillTarget,
};
#[cfg(feature = "revocable_store")]
use quanta_memory_platform_facade::{
    MemoryPlatformBuilder, RevocableStore, RoutedSpillController, SpillReceipt,
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
        eprintln!(
            "usage: spill_redb_crash_child <db-path> <fill <count>|stream|hole|store-spilled>"
        );
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
        #[cfg(feature = "revocable_store")]
        "store-spilled" => {
            let platform = MemoryPlatformBuilder::new()
                .with_preset(MemoryPreset::Development)
                .build()
                .expect("child platform");
            let store = platform
                .revocable_store_builder::<String, Vec<u8>>()
                .bind(StoreBindingHandle::new(
                    MemoryClass::ArtifactCache,
                    "durability-spill-child".to_string(),
                    None,
                ))
                .build()
                .expect("child store");
            let controller = RoutedSpillController::new(backing);
            store.insert_checked("k0".to_string(), payload(0)).expect("child insert");
            let receipt = store
                .spill_one_durable(&controller, |_key, value| Ok(value.clone()))
                .expect("child durable spill")
                .expect("one committed record");
            assert!(matches!(receipt, SpillReceipt::Durable { sequence: 0, .. }));
            let charged =
                platform.broker().ledger_snapshot().expect("child ledger").total_owner_bytes();
            assert_eq!(charged, 0, "source charge retired after durable commit");
            assert_eq!(controller.spilled_count().expect("durable inventory"), 1);
            writeln!(stdout, "CHARGE {charged}").expect("CHARGE line");
            writeln!(stdout, "READY").expect("READY line");
            stdout.flush().expect("flush");
            loop {
                std::thread::park();
            }
        }
        mode => {
            eprintln!("unknown mode: {mode}");
            std::process::exit(2);
        }
    }
}
