//! Plumbing shared by every Afixo service. Deliberately boring: nothing in here
//! knows about subjects, personas or rules. Domain logic lives in
//! `afixo-engine` and in the services themselves.

pub mod config;
pub mod db;
pub mod grpc;
pub mod ids;
pub mod telemetry;
pub mod time;
pub mod tokens;
