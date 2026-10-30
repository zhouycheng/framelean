//! ABI 1, media JSON schema 2. Only the isolated Host loads this library.
mod adaptation;
mod analysis;
mod api;
mod backends;
mod capabilities;
mod operations;
mod planning;
mod source;
mod validation;
pub use api::Operation;
