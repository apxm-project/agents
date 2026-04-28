//! Reserved flow names referenced by AIS handlers.

pub const MAIN: &str = "main";
pub const COMMUNICATE: &str = "communicate";
pub const DELEGATE: &str = "delegate";
pub const NEGOTIATE: &str = "negotiate";

pub const DELEGATE_FLOWS: &[&str] = &[DELEGATE, MAIN];
pub const NEGOTIATE_FLOWS: &[&str] = &[NEGOTIATE, COMMUNICATE, MAIN];
pub const HANDOFF_FLOWS: &[&str] = &[COMMUNICATE, MAIN];
pub const COMMUNICATE_FLOWS: &[&str] = &[COMMUNICATE, MAIN];
