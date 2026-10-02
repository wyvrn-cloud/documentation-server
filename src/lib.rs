//! A DIDComm v2 documentation registry: indexes DIDComm protocol definitions (in
//! didcomm.org's format), the DIDComm Messaging specification and JSON Schemas, and
//! serves them over `https://wyvrn.app/documentation/1.0`.

pub mod config;
pub mod index;
pub mod markdown;
pub mod registry;
pub mod server;
