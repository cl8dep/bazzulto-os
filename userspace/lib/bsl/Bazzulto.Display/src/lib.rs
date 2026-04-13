//! Bazzulto.Display — drawing API for applications.
//!
//! This crate is the public drawing interface for Bazzulto apps.
//! Apps draw into a `Surface` (their private canvas); the display server
//! composites it onto the physical framebuffer.
//!
//! # What is NOT here
//!
//! `FramebufferSurface` and `TextRenderer` live inside `bzdisplayd` — they
//! are implementation details of the display server, not part of the app API.
//! Apps never touch the framebuffer directly.

#![no_std]

extern crate alloc;

pub mod color;
pub mod ffi;
pub mod font_manager;
pub mod geometry;
pub mod screen;
pub mod surface;
