#![no_main]
use jj_compose_core::Draft;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. Convert raw random bytes to a string
    if let Ok(message) = std::str::from_utf8(data) {
        // 2. We don't care if parse() returns Ok(Draft) or Err(ParseError)
        // We only care that it NEVER panics/crashes!
        let result = Draft::parse(message);

        // 3. (Bonus Validation) If the fuzzer magically generated a string
        // that successfully parsed, ensure that `build()` also doesn't panic,
        // and that `render()` is truly lossless (which is your core design constraint).
        if let Ok(draft) = result {
            let _ = draft.build();
            assert_eq!(
                draft.render(),
                message.trim_end(),
                "Parse was successful, but render() failed the lossless guarantee!"
            );
        }
    }
});
