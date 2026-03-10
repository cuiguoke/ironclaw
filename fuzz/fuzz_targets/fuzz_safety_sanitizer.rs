#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

use ironclaw::safety::Sanitizer;

static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(Sanitizer::new);

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Exercise the main sanitization path
        let result = SANITIZER.sanitize(s);
        // Verify invariant: warnings should have valid ranges
        for w in &result.warnings {
            assert!(w.location.end <= s.len());
        }
        // Verify invariant: critical severity triggers modification
        let has_critical = result.warnings.iter().any(|w| {
            w.severity == ironclaw::safety::Severity::Critical
        });
        if has_critical {
            assert!(result.was_modified);
        }
    }
});
