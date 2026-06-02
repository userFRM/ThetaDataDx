//! Authentication for `ThetaData` direct server access.
//!
//! Two sub-modules handle the two halves of the auth story:
//!
//! - `creds` — Parse `creds.txt` (email + password)
//! - `nexus` — HTTP POST to Nexus API to obtain a session UUID
//!
//! # Auth flow
//!
//! ```text
//! creds.txt --> Credentials --> nexus::authenticate() --> AuthResponse.session_id
//!                                                           |
//!                         +---------------------------------+
//!                         |
//!             Historical channel:  session UUID attached to every request
//!             Streaming channel:   email + password sent in the login handshake
//! ```

pub(crate) mod creds;
pub(crate) mod nexus;
pub(crate) mod session;

pub use creds::Credentials;

// `authenticate` and the associated response types are session-internal:
// they support the `MddsClient` auth handshake and the CLI's explicit
// re-auth path. Both require `__internal` to be enabled; external crates
// working through `ThetaDataDxClient` never need to call `authenticate`
// directly — the client handles re-auth internally.
//
// In non-`__internal` builds, `authenticate_at` and `SessionToken` are
// `pub(crate)` only — invisible outside the crate.
#[cfg(feature = "__internal")]
pub use nexus::{authenticate, authenticate_at, AuthResponse, AuthUser};
#[cfg(feature = "__internal")]
pub use session::SessionToken;

#[cfg(not(feature = "__internal"))]
pub(crate) use nexus::authenticate_at;
#[cfg(not(feature = "__internal"))]
pub(crate) use session::SessionToken;
