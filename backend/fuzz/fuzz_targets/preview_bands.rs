#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // imgio's band decode of a large preview (bands.rs), on any stream: the
    // header and restart-marker walk, and the bands cut as thin as the
    // markers allow. Where they decode, they must be the same bytes as the
    // stream decoded whole; the check panics otherwise.
    genzo_imgio::check_preview_bands(data);
});
