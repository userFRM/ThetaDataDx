//! Authentication for `ThetaData` direct server access.
//!
//! Two sub-modules handle the two halves of the auth story:
//!
//! - [`creds`] — Parse `creds.txt` (email + password)
//! - [`nexus`] — HTTP POST to Nexus API to obtain a session UUID
//!
//! # Auth flow (from decompiled Java — `AuthenticationManager`)
//!
//! ```text
//! creds.txt --> Credentials --> nexus::authenticate() --> SessionToken
//!                                                           |
//!                         +---------------------------------+
//!                         |
//!             MDDS (gRPC): session UUID in QueryInfo.auth_token
//!             FPSS (TCP):  email + password sent over TCP handshake
//! ```

pub mod creds;
pub mod nexus;

pub use creds::Credentials;
pub use nexus::{authenticate, AuthResponse, AuthUser, SessionToken};
