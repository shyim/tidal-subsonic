//! Domain API methods for `TidalClient`, split into `impl` blocks by concern.
//! The shared plumbing (auth, `authenticated_get`, `api_get`) lives in the
//! parent `client` module and is `pub(super)`-visible to these blocks.

mod events;
mod library;
mod mixes;
mod streaming;
