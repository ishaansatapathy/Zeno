//! Checkout: turning "I want this course" into a provider order the browser can pay.
//!
//! The browser sends only a course id. The price, currency and title come from our database at
//! the moment the order is made and are frozen in the order, so nothing the client says can change
//! what is charged. Enrollment is never granted here; only a verified provider event does that.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
