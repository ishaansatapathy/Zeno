//! Paying creators what the ledger says they are owed.
//!
//! Zeno holds the money (decision D1), so a payout is a bank transfer an admin makes by hand in
//! the pilot. The software's job is to make that safe: a creator asks, the amount leaves their
//! balance in the ledger **at once** (so the same money cannot be asked for twice, however many
//! requests race), the money inside a refund window is never payable, and a payout that is cancelled
//! gives its money back by a new journal rather than by editing history.

pub mod dto;
pub mod repo;
pub mod routes;
pub mod service;
