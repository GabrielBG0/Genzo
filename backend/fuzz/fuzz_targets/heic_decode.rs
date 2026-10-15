#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Errors are expected; crashes, hangs and sanitizer reports are bugs.
    let _ = genzo_imgio::decode(data, genzo_imgio::DecodeOptions { threads: 1 });
    let _ = genzo_imgio::decode_preview(data);
});
