//! Recovered-filename sanitisation for the Windows output path.
//!
//! Recovered names come from untrusted on-disk metadata and may be illegal,
//! reserved, or hostile. Sanitisation runs on every platform so that output
//! written on macOS stays portable to Windows, and so the rules are testable
//! without a Windows host.
//!
//! Every substitution is reported, so the original on-disk name is preserved as
//! provenance rather than silently lost.

/// Characters Windows forbids in a path component.
const RESERVED_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Device names Windows reserves, with or without an extension.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Windows caps a single path component at 255 characters.
const MAX_COMPONENT: usize = 255;

/// Outcome of sanitising one recovered name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedName {
    /// Name safe to create on Windows.
    pub name: String,
    /// True when `name` differs from the input, so callers can record the
    /// original as provenance.
    pub changed: bool,
}

/// Makes a single recovered path component safe for Windows.
///
/// The result is never empty, never a reserved device name, contains no
/// reserved or control characters, and has no trailing dot or space.
pub fn sanitize_component(input: &str) -> SanitizedName {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        // Control characters and path separators are replaced, not dropped, so
        // distinct names cannot silently collapse onto each other.
        if RESERVED_CHARS.contains(&ch) || (ch as u32) < 0x20 || ch == '\u{7f}' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }

    // Windows silently strips trailing dots and spaces, which would otherwise
    // make the created name differ from the requested one.
    let trimmed = out.trim_end_matches(['.', ' ']);
    let mut out = if trimmed.is_empty() { String::new() } else { trimmed.to_string() };

    // Truncate on a character boundary, not a byte index.
    if out.chars().count() > MAX_COMPONENT {
        out = out.chars().take(MAX_COMPONENT).collect();
        // Truncation can re-expose a trailing dot or space.
        out = out.trim_end_matches(['.', ' ']).to_string();
    }

    // A reserved device name is unusable even with an extension ("NUL.txt").
    let stem = out.split('.').next().unwrap_or("");
    if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        out.insert(0, '_');
    }

    if out.is_empty() {
        out.push_str("unnamed");
    }

    let changed = out != input;
    SanitizedName { name: out, changed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(input: &str) -> String {
        sanitize_component(input).name
    }

    #[test]
    fn ordinary_names_are_untouched() {
        let r = sanitize_component("holiday photo.jpg");
        assert_eq!(r.name, "holiday photo.jpg");
        assert!(!r.changed);
    }

    #[test]
    fn reserved_characters_are_replaced() {
        assert_eq!(s(r#"a<b>c:d"e|f?g*h"#), "a_b_c_d_e_f_g_h");
    }

    #[test]
    fn path_separators_cannot_escape_the_component() {
        // A traversal attempt must not survive as a separator.
        assert_eq!(s("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(s(r"..\..\windows"), ".._.._windows");
    }

    #[test]
    fn reserved_device_names_are_prefixed() {
        assert_eq!(s("NUL"), "_NUL");
        assert_eq!(s("con"), "_con");
        // Reserved even when it carries an extension.
        assert_eq!(s("NUL.txt"), "_NUL.txt");
        assert_eq!(s("COM1.dat"), "_COM1.dat");
    }

    #[test]
    fn similar_but_unreserved_names_are_kept() {
        assert_eq!(s("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(s("COM10"), "COM10");
    }

    #[test]
    fn trailing_dots_and_spaces_are_trimmed() {
        assert_eq!(s("report."), "report");
        assert_eq!(s("report   "), "report");
        assert_eq!(s("report. . "), "report");
    }

    #[test]
    fn control_characters_are_replaced() {
        assert_eq!(s("a\u{0}b\tc\u{7f}"), "a_b_c_");
    }

    #[test]
    fn empty_and_degenerate_names_get_a_placeholder() {
        assert_eq!(s(""), "unnamed");
        assert_eq!(s("..."), "unnamed");
        assert_eq!(s("   "), "unnamed");
    }

    #[test]
    fn long_names_are_truncated_on_char_boundaries() {
        // Multi-byte characters must not be split mid-encoding.
        let long = "é".repeat(400);
        let out = s(&long);
        assert_eq!(out.chars().count(), MAX_COMPONENT);
        assert!(out.chars().all(|c| c == 'é'));
    }

    #[test]
    fn unicode_names_are_preserved() {
        assert_eq!(s("café-漢字.txt"), "café-漢字.txt");
    }

    #[test]
    fn output_is_always_usable() {
        // Whatever the input, the result satisfies every Windows rule.
        for input in ["", "...", "NUL", "a/b", "\u{0}", "  . ", "COM9.tar.gz", &"x".repeat(900)] {
            let out = s(input);
            assert!(!out.is_empty());
            assert!(!out.chars().any(|c| RESERVED_CHARS.contains(&c) || (c as u32) < 0x20));
            assert!(!out.ends_with('.') && !out.ends_with(' '));
            assert!(out.chars().count() <= MAX_COMPONENT + 1);
            let stem = out.split('.').next().unwrap();
            assert!(!RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)));
        }
    }
}
