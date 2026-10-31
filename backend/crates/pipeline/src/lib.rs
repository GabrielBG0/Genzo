//! The edit graph, its WGSL shaders and the tile scheduler.
//!
//! [`gpu`] holds the 0.D spike: the render thread and a minimal raw path
//! (linearize, white balance, RCD demosaic, mip pyramid, camera matrix,
//! exposure, output transform). The tiling is shared by every renderer, and
//! the CPU test pattern still feeds the 0.E webview spike.

pub mod budget;
pub mod camera_color;
pub mod geometry;
pub mod gpu;
pub mod histogram;
pub mod look;
pub mod picker;
pub mod raw;
pub mod reference;
pub mod test_pattern;
pub mod tiles;
