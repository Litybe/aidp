/// Normalize command output to make it comparable to insta snapshots by replacing
/// workspace paths, run IDs, and backslashes with consistent tokens.
///
/// Applies substitutions in order:
/// 1. Replace workspace path (both `/` and `\` variants) with `<WS>`
/// 2. Replace run IDs (format: 8 digits - 6 digits - 4 hex digits, e.g. `20260917-121500-a3f1`) with `<RUN_ID>`
/// 3. Replace remaining `\` with `/`
pub fn normalize(input: &str, ws: &crate::TempWorkspace) -> String {
    let mut result = String::with_capacity(input.len());
    let ws_path_str = ws.path().to_string_lossy();
    let ws_path_forward = ws_path_str.to_string();
    let ws_path_backslash = ws_path_str.replace('/', "\\");

    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Try to match workspace path (backslash version first, as it's more specific on Windows)
        if bytes[i..].starts_with(ws_path_backslash.as_bytes()) {
            result.push_str("<WS>");
            i += ws_path_backslash.len();
        } else if bytes[i..].starts_with(ws_path_forward.as_bytes()) {
            result.push_str("<WS>");
            i += ws_path_forward.len();
        } else if is_run_id_start(&bytes[i..]) {
            // Try to match a run ID pattern: YYYYMMDD-HHMMSS-XXXX (4 hex)
            if let Some(run_id_len) = try_match_run_id(&bytes[i..]) {
                result.push_str("<RUN_ID>");
                i += run_id_len;
            } else {
                // Decode a full UTF-8 character instead of a single byte
                if let Some(ch) = input[i..].chars().next() {
                    result.push(ch);
                    i += ch.len_utf8();
                } else {
                    i += 1;
                }
            }
        } else {
            // Decode a full UTF-8 character instead of a single byte
            if let Some(ch) = input[i..].chars().next() {
                result.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }

    // Replace any remaining backslashes with forward slashes
    result.replace('\\', "/")
}

/// Check if we might be at the start of a run ID (starts with 8 digits)
fn is_run_id_start(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[0..8].iter().all(|b| b.is_ascii_digit())
}

/// Try to match and consume a run ID pattern from the current position.
/// Format: 8 digits - 6 digits - 4 hex digits (lowercase)
/// Example: 20260917-121500-a3f1
/// Returns Some(length) if matched, None otherwise.
fn try_match_run_id(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 20 {
        // Minimum length: 8 + 1 + 6 + 1 + 4 = 20
        return None;
    }

    // Check pattern: DDDDDDDD-DDDDDD-XXXX
    // Position: 01234567 8 9ABCDE F GHIJ

    // First 8 digits
    if !bytes[0..8].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // First hyphen
    if bytes[8] != b'-' {
        return None;
    }

    // Next 6 digits
    if !bytes[9..15].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // Second hyphen
    if bytes[15] != b'-' {
        return None;
    }

    // Last 4 hex digits (case-insensitive match)
    if !bytes[16..20].iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }

    Some(20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_run_id_and_workspace() {
        let ws = crate::TempWorkspace::new("normalize_test");
        let ws_path = ws.path().to_string_lossy().to_string();

        // Create input with workspace path (using backslashes as on Windows)
        let input = format!(
            "Output from {}\\file.txt with run 20260917-121500-a3f1",
            ws_path.replace('/', "\\")
        );

        let normalized = normalize(&input, &ws);

        // Check that run ID and workspace path are replaced
        assert!(normalized.contains("<RUN_ID>"));
        assert!(normalized.contains("<WS>"));
        assert!(!normalized.contains("20260917-121500-a3f1"));
        assert!(!normalized.contains("\\"));
    }

    #[test]
    fn test_normalize_idempotent() {
        let ws1 = crate::TempWorkspace::new("normalize_test_1");
        let ws2 = crate::TempWorkspace::new("normalize_test_2");

        // Create inputs with different run IDs and workspace paths
        let input1 = format!(
            "Output from {} with run 20260917-121500-a3f1",
            ws1.path().display()
        );

        let input2 = format!(
            "Output from {} with run 20260918-143021-f8d2",
            ws2.path().display()
        );

        let norm1 = normalize(&input1, &ws1);
        let norm2 = normalize(&input2, &ws2);

        // Both should normalize to the same string since they have the same structure
        assert_eq!(norm1, norm2);
    }

    #[test]
    fn test_normalize_no_backslashes() {
        let ws = crate::TempWorkspace::new("normalize_test_no_backslash");

        let input = "C:\\some\\path\\file.txt and more\\backslashes";
        let normalized = normalize(input, &ws);

        // All backslashes should be replaced with forward slashes
        assert!(!normalized.contains('\\'));
        assert!(normalized.contains('/'));
    }

    #[test]
    fn test_run_id_matching() {
        // Valid run IDs
        assert_eq!(try_match_run_id(b"20260917-121500-a3f1"), Some(20));
        assert_eq!(try_match_run_id(b"20260917-121500-ABCD"), Some(20));
        assert_eq!(try_match_run_id(b"20260917-121500-0000"), Some(20));

        // Invalid run IDs
        assert_eq!(try_match_run_id(b"20260917-121500-a3f"), None); // Too short
        assert_eq!(try_match_run_id(b"2026091-121500-a3f1"), None); // Only 7 digits
        assert_eq!(try_match_run_id(b"20260917-12150-a3f1"), None); // Only 5 middle digits
        assert_eq!(try_match_run_id(b"20260917-121500-a3fg"), None); // 'g' is not hex
    }

    #[test]
    fn test_try_match_run_id_off_by_one() {
        // Regression test: 19 bytes (just under the 20-byte minimum) should not panic
        // This is exactly 19 bytes: "20260917-121500-a3f" (8 + 1 + 6 + 1 + 3 = 19)
        let nineteen_byte_slice = b"20260917-121500-a3f";
        assert_eq!(nineteen_byte_slice.len(), 19);
        assert_eq!(try_match_run_id(nineteen_byte_slice), None);
    }

    #[test]
    fn test_normalize_utf8_characters() {
        // Regression test: non-ASCII UTF-8 characters should be decoded as full chars,
        // not corrupted by byte-by-byte interpretation
        let ws = crate::TempWorkspace::new("normalize_utf8_test");

        // Input with non-ASCII UTF-8 characters (accented chars from French)
        let input = "Erreur: fichier non trouvé — café";
        let normalized = normalize(input, &ws);

        // The non-ASCII characters should survive in the output unmangled
        assert!(
            normalized.contains("café"),
            "UTF-8 character 'é' was corrupted"
        );
        assert!(
            normalized.contains("trouvé"),
            "UTF-8 character 'é' in 'trouvé' was corrupted"
        );
        assert!(normalized.contains('—'), "UTF-8 em-dash was corrupted");
    }
}
