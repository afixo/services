//! identity-service: subjects, personas and sensitivity-tiered fields.
//!
//! Built as a library so the integration tests in `tests/` can drive
//! [`server::IdentitySvc`] through the generated `IdentityService` trait
//! without a network; `main.rs` is the thin binary around it. See CLAUDE.md in
//! this directory for the invariants.

pub mod repo;
pub mod server;
