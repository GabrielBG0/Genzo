//! The edit graph, its WGSL shaders and the tile scheduler.
//!
//! The wgpu pipeline is the 0.D spike and waits for the M0 license review.
//! What exists now is the tiling, which every renderer shares, and a CPU test
//! pattern that feeds the 0.E pixels-through-the-webview spike.

pub mod test_pattern;
pub mod tiles;
