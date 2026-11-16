//! Replacement-server components, not a playable server. Connectionless queries,
//! offline BSP geometry (world plus static brush models), deterministic movement
//! primitives and bitstreams; no authentication/sign-on or client replication.
#![forbid(unsafe_code)]
pub mod bitstream;
pub mod bsp;
#[cfg(test)]
mod bsp_tests;
pub mod collision;
pub mod entities;
pub mod movement;
pub mod query;
pub mod spawn;
pub mod transport;
#[cfg(test)]
mod world_tests;
