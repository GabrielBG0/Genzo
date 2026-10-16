#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Errors are expected; crashes, hangs and huge allocations are bugs.
    let _ = genzo_imgio::dng::decode_linear(data);
});
