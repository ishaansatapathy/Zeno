//! The public face of the platform: what a visitor with no account can see.
//!
//! Everything here is a whitelisted projection. Handlers never return a database row, and the
//! queries only reach published, undeleted courses, so a draft or deleted course is
//! indistinguishable from one that never existed.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
