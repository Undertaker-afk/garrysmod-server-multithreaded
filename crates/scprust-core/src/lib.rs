//! Engine-independent server gameplay core. No Lua, sockets, unsafe code, or
//! engine pointers. Serde is optional; algorithms can be embedded independently.
#![forbid(unsafe_code)]
pub mod ai;
pub mod navigation;
pub mod world;

pub use ai::*;
pub use navigation::*;
pub use world::*;

pub const MAX_ENTITIES: usize = 8192;
pub const MAX_BATCH: usize = 128;
pub const MAX_COORD: f64 = 1_000_000.0;

pub fn valid_position(p: &[f64; 3]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

pub fn distance_squared(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b.iter()).map(|(a, b)| (a - b) * (a - b)).sum()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreError {
    Invalid(&'static str),
    Conflict,
    Unavailable,
}

pub type CoreResult<T> = Result<T, CoreError>;
