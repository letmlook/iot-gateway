//! Mitsubishi MC plugin runtime state.

use gateway_sdk::Group;

pub struct MCState {
    pub host: String,
    pub port: u16,
    pub network: u8,
    pub station: u8,
    pub groups: Vec<Group>,
    pub tags: Vec<gateway_sdk::Tag>,
}
