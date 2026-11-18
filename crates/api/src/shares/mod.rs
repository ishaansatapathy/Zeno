//! Share links: anyone with the link can watch a recording, no account needed.
//!
//! The owner makes links (optionally with a password and an expiry) and can revoke any of them.
//! A visitor resolves a link to what the page needs to draw itself (a cacheable, unauthenticated
//! read), then asks to play; that starts an ordinary playback session, tied to the link instead
//! of an account. From there the same token, playlist and key-server checks apply, and each of
//! them re-asks the database whether the link is still good, so revoking one takes effect on the
//! viewer's very next request.

pub mod code;
pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
