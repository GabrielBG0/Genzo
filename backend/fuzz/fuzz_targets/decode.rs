#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(
    init: {
        // As in dng_linear: panics raised in rawler are caught and turned
        // into errors (src/ljpeg.rs), so let them unwind to that handler.
        // Any other panic is still a crash.
        let abort = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let in_rawler = info.location().is_some_and(|l| l.file().contains("/rawler-"));
            if !in_rawler {
                abort(info);
            }
        }));
    },
    |data: &[u8]| {
        // The whole path the pipeline uses: sniffing, routing (DNGVersion),
        // the metadata reads and the decoders. Errors are expected; crashes,
        // hangs and huge allocations are bugs.
        let _ = genzo_imgio::decode(data, genzo_imgio::DecodeOptions { threads: 1 });
    }
);
