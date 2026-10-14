//! Plain data types shared by every backend crate, and the source of the
//! contract between the engine and the frontend.
//!
//! No IO lives here.

pub mod api;
pub mod contract;
pub mod frame;
pub mod typescript;

pub use contract::CONTRACT_VERSION;
