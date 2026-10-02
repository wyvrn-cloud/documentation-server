//! A DIDComm documentation registry: indexes DIDComm v1 and v2 protocol definitions
//! (didcomm.org's format and the Aries RFCs), the DIDComm Messaging specification and
//! its extensions, and JSON Schemas, and serves them over
//! `https://wyvrn.app/documentation/1.1` (and 1.0).

pub mod aries;
pub mod config;
pub mod index;
pub mod markdown;
pub mod registry;
pub mod server;
