#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

use ironclaw::safety::LeakDetector;

static DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Exercise scan path
        let result = DETECTOR.scan(s);
        // Invariant: if should_block, there must be matches
        if result.should_block {
            assert!(!result.matches.is_empty());
        }
        // Invariant: match locations must be valid
        for m in &result.matches {
            assert!(m.location.end <= s.len());
        }

        // Exercise scan_and_clean path
        let _ = DETECTOR.scan_and_clean(s);
    }
});
