//! TIDAL API client, split into data types, the HTTP client, DASH parsing, and
//! domain API methods (under `api`, as extra `impl TidalClient` blocks).

mod api;
pub mod client;
pub(crate) mod dash;
pub mod types;

pub use client::*;
pub use types::*;
