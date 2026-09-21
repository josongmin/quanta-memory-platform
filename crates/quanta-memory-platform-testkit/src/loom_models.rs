//! Exhaustive small-state concurrency models for cross-owner invariants.
//!
//! These models deliberately encode the same authority transitions as the
//! production owners while keeping the state space small enough for Loom to
//! enumerate. They prove atomic postconditions; they do not claim to execute
//! production mutex or storage implementations under Loom instrumentation.

use loom::sync::{Arc, Mutex};
use loom::thread;

#[derive(Debug)]
struct BudgetState {
    used: u64,
    hard: u64,
}

#[test]
fn loom_reserve_vs_reconfigure_preserves_installed_cap() {
    loom::model(|| {
        let state = Arc::new(Mutex::new(BudgetState { used: 4, hard: 8 }));
        let reserve_state = Arc::clone(&state);
        let reserve = thread::spawn(move || {
            let mut state = reserve_state.lock().unwrap();
            if state.used.checked_add(3).is_some_and(|after| after <= state.hard) {
                state.used += 3;
                true
            } else {
                false
            }
        });
        let reconfigure_state = Arc::clone(&state);
        let reconfigure = thread::spawn(move || {
            let mut state = reconfigure_state.lock().unwrap();
            if state.used <= 5 {
                state.hard = 5;
                true
            } else {
                false
            }
        });
        let _ = reserve.join().unwrap();
        let _ = reconfigure.join().unwrap();
        let state = state.lock().unwrap();
        assert!(state.used <= state.hard);
    });
}

#[derive(Debug)]
struct TicketState {
    live: [bool; 2],
    used: u64,
}

fn release_one(state: &mut TicketState, index: usize, bytes: u64) -> bool {
    if !state.live[index] {
        return false;
    }
    state.live[index] = false;
    state.used = state.used.checked_sub(bytes).expect("live ticket owns its bytes");
    true
}

#[test]
fn loom_ticket_transition_has_single_terminal_owner() {
    loom::model(|| {
        let state = Arc::new(Mutex::new(TicketState { live: [true, true], used: 7 }));
        let single_state = Arc::clone(&state);
        let single = thread::spawn(move || release_one(&mut single_state.lock().unwrap(), 0, 3));
        let batch_state = Arc::clone(&state);
        let batch = thread::spawn(move || {
            let mut state = batch_state.lock().unwrap();
            if state.live.iter().all(|live| *live) {
                state.live = [false, false];
                state.used = state.used.checked_sub(7).expect("batch owns both tickets");
                true
            } else {
                false
            }
        });
        let _ = single.join().unwrap();
        let batch_succeeded = batch.join().unwrap();

        let mut state = state.lock().unwrap();
        if !batch_succeeded {
            let _ = release_one(&mut state, 1, 4);
        }
        assert_eq!(state.live, [false, false]);
        assert_eq!(state.used, 0);
    });
}

#[test]
fn loom_close_vs_mutation_never_mutates_after_close() {
    loom::model(|| {
        let state = Arc::new(Mutex::new((false, 0u64)));
        let close_state = Arc::clone(&state);
        let close = thread::spawn(move || {
            close_state.lock().unwrap().0 = true;
        });
        let mutation_state = Arc::clone(&state);
        let mutation = thread::spawn(move || {
            let mut state = mutation_state.lock().unwrap();
            if state.0 {
                false
            } else {
                state.1 += 1;
                true
            }
        });
        close.join().unwrap();
        let mutation_succeeded = mutation.join().unwrap();
        let state = state.lock().unwrap();
        assert!(state.0);
        assert_eq!(state.1, u64::from(mutation_succeeded));
        if state.0 {
            let before = state.1;
            assert_eq!(state.1, before, "closed owner rejects subsequent mutation");
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PayloadOwner {
    Memory,
    Spill,
}

#[test]
fn loom_spill_transfer_never_has_zero_or_double_owner() {
    loom::model(|| {
        let state = Arc::new(Mutex::new((true, PayloadOwner::Memory)));
        let spill_state = Arc::clone(&state);
        let spill = thread::spawn(move || {
            let mut state = spill_state.lock().unwrap();
            if state.0 && state.1 == PayloadOwner::Memory {
                state.1 = PayloadOwner::Spill;
            }
        });
        let restore_state = Arc::clone(&state);
        let offline_restore = thread::spawn(move || {
            let mut state = restore_state.lock().unwrap();
            state.0 = false;
            if state.1 == PayloadOwner::Spill {
                state.1 = PayloadOwner::Memory;
            }
        });
        spill.join().unwrap();
        offline_restore.join().unwrap();
        let state = state.lock().unwrap();
        assert!(matches!(state.1, PayloadOwner::Memory | PayloadOwner::Spill));
        if !state.0 {
            assert_eq!(state.1, PayloadOwner::Memory, "offline spill cannot retain ownership");
        }
    });
}

#[derive(Debug)]
struct CallbackState {
    used: u64,
    live: bool,
    version: u64,
}

#[test]
fn loom_versioned_callback_compensation_is_exactly_once() {
    loom::model(|| {
        let state = Arc::new(Mutex::new(CallbackState { used: 5, live: true, version: 1 }));
        let callback_state = Arc::clone(&state);
        let callback = thread::spawn(move || {
            let observed_version = callback_state.lock().unwrap().version;
            thread::yield_now(); // external callback executes outside the owner lock
            let mut state = callback_state.lock().unwrap();
            if state.live && state.version == observed_version {
                state.live = false;
                state.used = state.used.checked_sub(5).expect("compensation owns live bytes");
                state.version += 1;
                true
            } else {
                false
            }
        });
        let release_state = Arc::clone(&state);
        let release = thread::spawn(move || {
            let mut state = release_state.lock().unwrap();
            if state.live {
                state.live = false;
                state.used = state.used.checked_sub(5).expect("release owns live bytes");
                state.version += 1;
                true
            } else {
                false
            }
        });
        let callback_won = callback.join().unwrap();
        let release_won = release.join().unwrap();
        assert_ne!(callback_won, release_won, "exactly one terminal transition wins");
        let state = state.lock().unwrap();
        assert!(!state.live);
        assert_eq!(state.used, 0);
        assert_eq!(state.version, 2);
    });
}
