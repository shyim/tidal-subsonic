//! TIDAL API client, split into data types, the HTTP client, and DASH parsing.

pub mod client;
pub(crate) mod dash;
pub mod types;

pub use client::*;
pub use types::*;
