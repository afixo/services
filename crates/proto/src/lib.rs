//! The Afixo wire contract: every message and service defined under
//! `proto/afixo/v1`, generated at build time by `tonic-prost-build`.
//!
//! Nothing in this crate is hand-written except this file. Change the `.proto`
//! files, not the generated code.

#![allow(clippy::pedantic, clippy::all, missing_debug_implementations)]

pub mod v1 {
    tonic::include_proto!("afixo.v1");
}

pub use v1::*;

/// Package-qualified gRPC service names, as they appear in health checks and reflection.
pub mod names {
    pub const AUTH: &str = "afixo.v1.AuthService";
    pub const IDENTITY: &str = "afixo.v1.IdentityService";
    pub const POLICY: &str = "afixo.v1.PolicyService";
    pub const DISCLOSURE: &str = "afixo.v1.DisclosureService";
    pub const AUDIT: &str = "afixo.v1.AuditService";
}
