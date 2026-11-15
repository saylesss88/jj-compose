#![no_main]
use libfuzzer_sys::fuzz_target;
// Adjust this import if Patch is inside a module like `jj_compose_core::patch::Patch`
use jj_compose_core::Patch;

fuzz_target!(|data: &[u8]| {
    // jj diffs are generally valid UTF-8 strings
    if let Ok(diff_text) = std::str::from_utf8(data) {
        // This is where panics would happen if your @@ header parsing
        // or hunk line counting has an out-of-bounds slice or unwrap()
        let _ = Patch::from_text(diff_text);
    }
});
