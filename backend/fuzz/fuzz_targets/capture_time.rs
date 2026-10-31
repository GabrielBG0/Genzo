#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // imgio::capture::capture on any input: the format sniff, the EXIF
    // locations per format (JPEG APP1, TIFF IFDs, CR3 boxes, the RAF header's
    // JPEG, PNG eXIf, HEIC through libheif), the date parse and the shooting
    // values (DEV-03). An empty capture is expected; crashes, hangs and huge
    // allocations are bugs.
    let c = genzo_imgio::capture::capture(data);
    assert_eq!(genzo_imgio::capture::capture_time(data), c.time);
    for r in [c.shooting.exposure_time, c.shooting.f_number, c.shooting.focal_length]
        .into_iter()
        .flatten()
    {
        assert!(r.num > 0 && r.den > 0, "{r:?}");
    }
    assert_ne!(c.shooting.iso, Some(0));
});
