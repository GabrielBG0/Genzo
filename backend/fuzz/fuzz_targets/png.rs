#![no_main]

use genzo_imgio::{DecodeOptions, FileFormat, decode, sniff};
use libfuzzer_sys::fuzz_target;

fuzz_target!(
    init: {
        // As in decode: panics raised in rawler (which sees every non-DNG
        // TIFF first) are caught and turned into errors, so let them unwind
        // to that handler. Any other panic is still a crash.
        let abort = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
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
        // The png decoder through imgio::decode; other formats are left to
        // their own targets. Errors are expected; crashes, hangs and huge
        // allocations are bugs.
        if sniff(data) == Some(FileFormat::Png) {
            let _ = decode(data, DecodeOptions { threads: 1 });
        }
    }
);
