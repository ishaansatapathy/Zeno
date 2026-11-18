//! Recordings: a person's library of video, whatever made it (an upload, the browser extension,
//! the desktop studio, a live stream or a meeting).
//!
//! A recording is a thin row on top of a video. The heavy part (upload, encode, protect, play)
//! is the video pipeline that already exists; this is the small, cheap layer people actually
//! name, find, share and delete.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
