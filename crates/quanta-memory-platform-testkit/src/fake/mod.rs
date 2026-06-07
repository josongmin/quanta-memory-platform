//! Fake broker / runtime substrate.
//!
//! `fake/*` owns the in-process fake broker and runtime-view fakes. It is the
//! only place a reservation/accounting model is reimplemented for tests, and it
//! must obey the SAME accounting invariants as the real core broker. Nothing
//! here is a production type.

pub mod broker;
pub mod views;

pub use broker::{
    AccountingFidelity, FakeBrokerFactory, FakeBrokerSnapshot, FakeProcessBroker, FakeReservation,
    FakeUsage,
};
pub use views::FakeRuntimeViewFactory;
