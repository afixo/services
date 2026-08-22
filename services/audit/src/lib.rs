//! audit-service: the append-only, hash-chained record of every disclosure
//! decision. `main.rs` boots it; the library exists so the integration tests
//! in `tests/` can drive [`server::AuditSvc`] through the `AuditService` trait.

pub mod chain;
pub mod repo;
pub mod server;
