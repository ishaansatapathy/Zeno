//! Coupons: a creator's launch offers.
//!
//! The discount is worked out by the server when the order is made and frozen in the order, so
//! nothing the browser says can change the price. Using a coupon is a conditional database update,
//! so a limit of ten redemptions holds even when a hundred buyers arrive at once; a redemption on
//! an order that expires unpaid is given back.

pub mod dto;
pub mod pricing;
pub mod repo;
pub mod routes;
pub mod service;
