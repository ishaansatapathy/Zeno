//! Payment webhooks: the only way money turns into access.
//!
//! The provider tells us what happened; we check that it is really the provider (signature over
//! the raw bytes), that it is about one of our orders and says what we expect (amount, currency),
//! and only then change anything. Everything an event does happens in one transaction together
//! with the record that the event was received, so a crash leaves nothing half done and a
//! redelivery finishes the job exactly once.
//!
//! `settlement` holds the state changes as plain functions over a connection. The webhook calls
//! them, and so will anything else that learns about a payment another way (reconciliation).

pub mod event;
pub mod repo;
pub mod routes;
pub mod service;
pub mod settlement;
