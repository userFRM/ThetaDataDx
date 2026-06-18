//! Authentication for `ThetaData` direct server access.
//!
//! Two sub-modules handle the two halves of the auth story:
//!
//! - `creds` — Source credentials: a `creds.txt` email + password pair,
//!   or an API key (inline or from `THETADATA_API_KEY`)
//! - `nexus` — HTTP POST to Nexus API to obtain a session UUID
//!
//! A credential carries one of two methods — email + password or an API
//! key — and both channels accept either.
//!
//! # Auth flow
//!
//! ```text
//! creds.txt / API key --> Credentials --> nexus::authenticate() --> AuthResponse.session_id
//!                                                                     |
//!                                  +----------------------------------+
//!                                  |
//!             Historical channel:  session UUID attached to every request
//!             Streaming channel:   the credential is sent in the login handshake
//! ```

pub(crate) mod creds;
pub(crate) mod nexus;
pub(crate) mod session;

pub use creds::Credentials;

// `authenticate` and the associated response types are session-internal:
// they support the `HistoricalClient` auth handshake and the CLI's explicit
// re-auth path. Both require `__internal` to be enabled; external crates
// working through `Client` never need to call `authenticate`
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
