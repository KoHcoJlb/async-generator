#![doc = include_str!("../README.md")]

mod generator;
mod yielder;

#[cfg(feature = "macros")]
pub use async_generator_macros::{generator, try_generator};
pub use generator::{Generator, TryGenerator, generator, try_generator};
