//! Replacement-server components, not a playable server. Connectionless queries,
//! offline BSP world-brush geometry and bitstreams; no authentication/sign-on.
#![forbid(unsafe_code)]
pub mod bitstream;
pub mod bsp;
#[cfg(test)]
mod bsp_tests;
pub mod collision;
pub mod entities;
pub mod query;
pub mod transport;
