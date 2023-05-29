//! ### `sea-streamer-runtime`: Async runtime abstraction
//!
//! This crate provides a small set of functions aligning the type signatures between `async-std` and `tokio`,
//! so that you can build applications generic to both runtimes.

#[cfg(all(feature = "runtime-async-std", feature = "runtime-tokio"))]
compile_error!("'runtime-async-std' and 'runtime-tokio' cannot be enabled at the same time");

#[cfg(feature = "file")]
pub mod file;
mod sleep;
mod task;
mod timeout;
mod mutex;

pub use sleep::*;
pub use task::*;
pub use timeout::*;
pub use mutex::*;