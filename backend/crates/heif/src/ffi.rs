//! Declarations for shim/shim.c. Only ints, sizes, byte pointers and opaque
//! handles cross this boundary; no libheif struct is mirrored in Rust.

use std::ffi::{c_char, c_int};

#[repr(C)]
pub struct Context {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Handle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Image {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub fn gz_init();
    pub fn gz_version() -> *const c_char;

    pub fn gz_context_new(max_threads: c_int) -> *mut Context;
    pub fn gz_context_free(ctx: *mut Context);
    pub fn gz_context_read(
        ctx: *mut Context,
        data: *const u8,
        len: usize,
        message: *mut *const c_char,
    ) -> c_int;
    pub fn gz_primary_handle(
        ctx: *mut Context,
        out: *mut *mut Handle,
        message: *mut *const c_char,
    ) -> c_int;

    pub fn gz_handle_free(h: *mut Handle);
    pub fn gz_handle_info(
        h: *const Handle,
        width: *mut c_int,
        height: *mut c_int,
        has_alpha: *mut c_int,
        luma_bits: *mut c_int,
    );
    pub fn gz_color(
        h: *const Handle,
        primaries: *mut c_int,
        transfer: *mut c_int,
        matrix: *mut c_int,
        full_range: *mut c_int,
    ) -> c_int;
    pub fn gz_icc_size(h: *const Handle) -> usize;
    pub fn gz_icc_copy(h: *const Handle, out: *mut u8, message: *mut *const c_char) -> c_int;
    pub fn gz_exif_size(h: *const Handle, id: *mut u32) -> usize;
    pub fn gz_exif_copy(
        h: *const Handle,
        id: u32,
        out: *mut u8,
        message: *mut *const c_char,
    ) -> c_int;
    pub fn gz_thumbnail(h: *const Handle) -> *mut Handle;

    pub fn gz_decode(
        h: *const Handle,
        wide: c_int,
        out: *mut *mut Image,
        message: *mut *const c_char,
    ) -> c_int;
    pub fn gz_image_free(img: *mut Image);
    pub fn gz_image_pixels(
        img: *const Image,
        width: *mut c_int,
        height: *mut c_int,
        stride: *mut usize,
        bits: *mut c_int,
    ) -> *const u8;
}
