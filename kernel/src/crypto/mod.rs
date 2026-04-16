// crypto/mod.rs — Cryptographic primitives for the kernel.
//
// All implementations are pure Rust, no_std, no external dependencies.

pub mod sha256;

pub use sha256::{Sha256, sha256, hex_digest};
