#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

use ironclaw::safety::Validator;

static VALIDATOR: LazyLock<Validator> = LazyLock::new(Validator::new);

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Exercise input validation
        let result = VALIDATOR.validate(s);
        // Invariant: empty input is always invalid
        if s.is_empty() {
            assert!(!result.is_valid);
        }

        // Exercise tool parameter validation with arbitrary JSON
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            let _ = VALIDATOR.validate_tool_params(&value);
        }
    }
});
