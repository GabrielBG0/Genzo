#![no_main]

use genzo_color::encoding::InputEncoding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // An embedded ICC profile as stage 4 reads it (moxcms underneath).
    // Errors are expected; crashes, hangs, huge allocations and a result
    // with non-finite numbers are bugs.
    if let Ok(e) = InputEncoding::icc(data) {
        assert!(e.to_working.iter().flatten().all(|v| v.is_finite()));
        assert!(e.lut().iter().all(|v| v.is_finite()));
    }
});
