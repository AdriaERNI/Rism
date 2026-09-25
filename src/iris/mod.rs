//! Atelier REST client layer. No tool semantics here — just faithful calls
//! to the IRIS Atelier API (see documentation/iris-atelier.md for the endpoint
//! map and every verified wire quirk this layer encodes).

pub mod compile;
pub mod documents;
pub mod http;
pub mod monitor;
pub mod serverinfo;
pub mod sql;
pub mod terminal;

pub use http::IrisClient;
