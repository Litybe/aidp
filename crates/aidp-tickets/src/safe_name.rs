//! Re-export of [`aidp_core::safe_name`], where the `key`/`service` shape
//! validation now lives (LOC-27). Kept so `crate::safe_name::check` resolves.

pub use aidp_core::safe_name::*;
