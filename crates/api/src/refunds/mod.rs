//! Refunds: asked for by students inside the window their course promised, decided by the
//! creator (or an admin), made by the payment provider, and final only when the provider says so.
//!
//! The money side is never optimistic. Approving a refund hands it to the provider with our own
//! refund id as the idempotency key, so asking twice (a retry after a timeout, the reconciliation
//! job) cannot refund twice. Access ends, and the order becomes `refunded`, in the same
//! transaction that records the provider's confirmation, whether that arrives as a webhook or is
//! found by reconciliation.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
