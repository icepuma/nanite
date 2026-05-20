//! Shared helpers for nanite clone plugins.
//!
//! Kept intentionally small — most plugin logic differs per API
//! (GitHub Link header vs. GitLab X-Next-Page, different auth header
//! names, different JSON shapes). Only truly reusable primitives
//! live here.

#![no_std]

extern crate alloc;

use alloc::string::String;

/// Returns `true` if the GitHub-style `Link` header indicates there is
/// a next page (i.e. contains a `rel="next"` segment).
///
/// Example: `<https://api.github.com/...&page=2>; rel="next", <...>; rel="last"`.
#[must_use]
pub fn has_next_link(link_header: Option<&str>) -> bool {
    let Some(header) = link_header else {
        return false;
    };
    for segment in header.split(',') {
        if segment.contains("rel=\"next\"") || segment.contains("rel=next") {
            return true;
        }
    }
    false
}

/// Percent-encodes a path segment the way the GitLab API expects for
/// group/project paths (forward slashes become `%2F`). Other characters
/// permitted in URL paths are left alone.
#[must_use]
pub fn encode_path_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(upper_hex(byte >> 4));
                out.push(upper_hex(byte & 0x0F));
            }
        }
    }
    out
}

fn upper_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + nibble - 10) as char,
        _ => '0',
    }
}

/// Builds a single-line user-agent string for nanite plugins.
#[must_use]
pub fn user_agent(plugin: &str) -> String {
    let mut s = String::from("nanite-plugin/");
    s.push_str(plugin);
    s.push('/');
    s.push_str(env_pkg_version());
    s
}

const fn env_pkg_version() -> &'static str {
    "0.1.0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_next_link_detects_next_rel() {
        assert!(has_next_link(Some(
            "<https://api.github.com/x?page=2>; rel=\"next\", <https://api.github.com/x?page=5>; rel=\"last\""
        )));
        assert!(has_next_link(Some("<https://api.github.com/x?page=2>; rel=next")));
    }

    #[test]
    fn has_next_link_returns_false_without_next() {
        assert!(!has_next_link(Some(
            "<https://api.github.com/x?page=1>; rel=\"prev\", <https://api.github.com/x?page=1>; rel=\"first\""
        )));
        assert!(!has_next_link(None));
        assert!(!has_next_link(Some("")));
    }

    #[test]
    fn encodes_slash_in_group_path() {
        assert_eq!(encode_path_segment("group/sub/sub"), "group%2Fsub%2Fsub");
    }

    #[test]
    fn passes_unreserved_chars_through() {
        assert_eq!(encode_path_segment("a-b_c.d~e"), "a-b_c.d~e");
    }
}
