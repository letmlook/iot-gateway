//! Alarm event lifecycle domain.

pub mod handlers;
pub mod store;

pub use store::{AlarmEvent, AlarmSeverity, AlarmStatus, AlarmStore};
