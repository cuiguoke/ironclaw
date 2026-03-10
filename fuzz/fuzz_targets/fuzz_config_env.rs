#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

use ironclaw::safety::{LeakDetector, Sanitizer, Validator};

static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(Sanitizer::new);
static VALIDATOR: LazyLock<Validator> = LazyLock::new(Validator::new);
static LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Exercise Sanitizer: detect and neutralize prompt injection attempts.
        let sanitized = SANITIZER.sanitize(input);
        // If no modification occurred, content must equal input.
        if !sanitized.was_modified {
            assert_eq!(sanitized.content, input);
        }

        // Exercise Validator: input validation (length, encoding, patterns).
        let result = VALIDATOR.validate(input);
        // ValidationResult must always be well-formed: if valid, no errors.
        if result.is_valid {
            assert!(
                result.errors.is_empty(),
                "valid result should have no errors"
            );
        }

        // Exercise LeakDetector: secret detection (API keys, tokens, etc.).
        let scan = LEAK_DETECTOR.scan(input);
        // scan_and_clean must not panic and must return valid UTF-8.
        let cleaned = LEAK_DETECTOR.scan_and_clean(input);
        // If scan found no matches, scan_and_clean should return the input unchanged.
        if scan.matches.is_empty() {
            if let Ok(ref clean_str) = cleaned {
                assert_eq!(
                    clean_str, input,
                    "scan_and_clean changed content despite no matches"
                );
            }
        }
    }
});
