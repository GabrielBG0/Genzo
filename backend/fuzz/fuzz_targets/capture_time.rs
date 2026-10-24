#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // imgio::capture::capture_time on any input: the format sniff, the EXIF
    // locations per format (JPEG APP1, TIFF IFDs, CR3 boxes, the RAF header's
    // JPEG, PNG eXIf, HEIC through libheif) and the date parse. None is
    // expected; crashes, hangs and huge allocations are bugs.
    let _ = genzo_imgio::capture::capture_time(data);
});
