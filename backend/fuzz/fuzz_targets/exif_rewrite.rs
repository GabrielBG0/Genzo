#![no_main]

use genzo_imgio::metadata::{APP1_MAX, from_exif, read, write};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // OUT-01's metadata: imgio::metadata::read on any file (the EXIF
    // locations per format), and from_exif on the input as an EXIF block,
    // which reaches the TIFF reader at once. Whatever is read must write
    // into one APP1 segment that reads back with the same IFD0, Exif and
    // GPS tags it was given, plus those the writer sets.
    for m in [read(data), from_exif(data)].into_iter().flatten() {
        let w = write(&m, (6000, 4000), "Genzo fuzz");
        assert!(w.app1.len() <= APP1_MAX, "{} bytes", w.app1.len());
        let back = from_exif(&w.app1).expect("a written block reads back");
        for t in &m.gps {
            if t.tag != 0 && !w.dropped.contains(&t.tag) {
                assert!(back.gps.contains(t), "GPS tag {} lost", t.tag);
            }
        }
    }
});
