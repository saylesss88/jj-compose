#![no_main]
use jj_compose_core::Draft;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. Convert the random bytes into a valid UTF-8 string
    if let Ok(text) = std::str::from_utf8(data) {
        // 2. Feed it to your parser. We don't care if it returns Ok or Err,
        // we only care that it doesn't panic!
        let _ = Draft::parse(text);
    }
});
