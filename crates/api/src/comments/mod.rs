//! Comments and reactions on a recording, left by the people watching it.
//!
//! A comment is words, optionally pinned to a moment of the video; a reaction is one emoji from a short list,
//! always pinned to a moment. They belong to the recording (revoking a link does not take the conversation
//! with it). Whoever may watch may read them, with one exception: a link with a password shows nothing, not
//! even this, to somebody who has not given the password (they prove it with the viewing's own token).
//! Only a signed-in person can leave one, so there is someone behind every comment, and the author and the
//! recording's owner can remove it.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
