#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(
    init: {
        // The DNG reader runs rawler's lossless JPEG decompressor inside
        // catch_unwind and turns its panics (rawler is not hardened against
        // bad input) into errors. libfuzzer-sys aborts on every panic, caught
        // or not, so let panics raised in rawler unwind to that handler. Any
        // other panic, and any that escaped the handler, is still a crash.
        let abort = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // rawler from crates.io (`rawler-0.8.0/`) or Genzo's patched
            // copy (`vendor/rawler/`).
            let in_rawler = info.location().is_some_and(|l| {
                let file = l.file().replace(std::path::MAIN_SEPARATOR, "/");
                file.contains("/rawler-") || file.contains("vendor/rawler/")
            });
            if !in_rawler {
                abort(info);
            }
        }));
    },
    |data: &[u8]| {
        // Errors are expected; crashes, hangs and huge allocations are bugs.
        let _ = genzo_imgio::dng::decode_linear(data);
    }
);
