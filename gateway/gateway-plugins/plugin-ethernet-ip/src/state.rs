//! EtherNet/IP plugin runtime state.

use gateway_sdk::Group;

pub struct EIPState {
    pub host: String,
    pub port: u16,
    pub groups: Vec<Group>,
    pub tags: Vec<gateway_sdk::Tag>,
}
