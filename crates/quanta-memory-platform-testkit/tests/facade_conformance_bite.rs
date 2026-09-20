//! QMP-040 lifecycle conformance must reject broken implementations, not just
//! accept the five real broker-backed stores. Public accounting callbacks no
//! longer exist, so faults are injected at the backend-neutral target seam.
#![cfg(feature = "fixtures")]

use quanta_memory_platform_testkit::conformance::suites::{
    run_store_lifecycle_suite, StoreLifecycleConformanceTarget,
};
use quanta_memory_platform_testkit::{ConformanceVerdict, ContractFamily, MemoryTestKit};

#[derive(Clone, Copy, Debug)]
enum Fault {
    None,
    LeakingCloseFake,
    StateLosingCloseFake,
    DoubleRefundFake,
}

#[derive(Debug)]
struct LifecycleFake {
    fault: Fault,
    retained: u64,
    ledger: i64,
    closed: bool,
    rebuild: bool,
    deferred_drop: bool,
}

impl LifecycleFake {
    fn new(fault: Fault) -> Self {
        Self { fault, retained: 0, ledger: 0, closed: false, rebuild: false, deferred_drop: false }
    }
}

impl StoreLifecycleConformanceTarget for LifecycleFake {
    fn populate(&mut self) -> Result<(), String> {
        self.retained = 21;
        self.ledger = 21;
        Ok(())
    }

    fn retained_bytes(&self) -> Result<u64, String> {
        Ok(self.retained)
    }

    fn ledger_bytes(&self) -> Result<u64, String> {
        u64::try_from(self.ledger).map_err(|_| format!("negative broker ledger: {}", self.ledger))
    }

    fn close(&mut self) -> Result<(), String> {
        if self.rebuild {
            if matches!(self.fault, Fault::StateLosingCloseFake) {
                self.retained = 0; // broken: failed close discards live state.
            }
            return Err("rebuild fence".to_string());
        }
        if self.closed {
            if matches!(self.fault, Fault::DoubleRefundFake) {
                self.ledger -= 21; // broken: duplicate close refunds twice.
            }
            return Ok(());
        }
        self.retained = 0;
        self.closed = true;
        if !matches!(self.fault, Fault::LeakingCloseFake) {
            self.ledger -= 21;
        }
        Ok(())
    }

    fn post_close_mutation_is_typed_closed(&mut self) -> bool {
        self.closed
    }

    fn begin_rebuild(&mut self) -> Result<(), String> {
        self.rebuild = true;
        Ok(())
    }

    fn finish_rebuild(&mut self) -> Result<(), String> {
        self.rebuild = false;
        if self.deferred_drop {
            self.ledger -= 21;
            self.deferred_drop = false;
        }
        Ok(())
    }

    fn drop_store(&mut self) {
        self.retained = 0;
        if self.rebuild {
            self.deferred_drop = true;
        } else if !matches!(self.fault, Fault::LeakingCloseFake) {
            self.ledger -= 21;
        }
        self.closed = true;
    }
}

fn verdict(fault: Fault) -> ConformanceVerdict {
    ConformanceVerdict::from_assertions(&run_store_lifecycle_suite("fake", || {
        Ok::<_, String>(LifecycleFake::new(fault))
    }))
}

#[test]
fn honest_fake_and_all_five_real_stores_pass_lifecycle_oracle() {
    assert!(verdict(Fault::None).passed());
    let real = MemoryTestKit::ci().run_facade_store_conformance();
    assert!(real.passed(), "real five-store suite: {real:?}");
}

#[test]
fn lifecycle_oracle_rejects_leaking_close_fake() {
    let result = verdict(Fault::LeakingCloseFake);
    assert!(!result.passed(), "leaking close/drop must be rejected");
    assert!(result.violated_families().contains(&ContractFamily::ReclaimAccounting));
}

#[test]
fn lifecycle_oracle_rejects_state_losing_close_fake() {
    let result = verdict(Fault::StateLosingCloseFake);
    assert!(!result.passed(), "failed close discarding entries must be rejected");
    assert!(result.violated_families().contains(&ContractFamily::ReclaimAccounting));
}

#[test]
fn lifecycle_oracle_rejects_double_refund_fake() {
    let result = verdict(Fault::DoubleRefundFake);
    assert!(!result.passed(), "double refund must be rejected");
    assert!(result.violated_families().contains(&ContractFamily::ReclaimAccounting));
}
