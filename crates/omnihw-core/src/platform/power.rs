//! Power settings for the Power page. Not read on Linux yet: everything returns empty data.

use crate::power::{Action, ApplyError, PowerInfo};

pub fn gather_settings(_p: &mut PowerInfo) {}

pub fn gather_devices(_p: &mut PowerInfo) {}

pub fn apply(_a: &Action) -> Result<(), ApplyError> {
    Err(ApplyError::Failed("Not supported on Linux yet".into()))
}

pub fn query_admin_raw() -> Option<(String, String)> {
    None
}
