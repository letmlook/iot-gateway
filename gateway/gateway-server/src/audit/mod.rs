//! Audit event domain.

pub mod handlers;
pub mod store;

pub use store::{AuditEvent, AuditStore};
