//! Reverse domain name notation helpers.
//!
//! Common Crawl stores domains as `com.example` instead of `example.com`.

/// Convert a normal domain (`example.com`) to reverse notation (`com.example`).
///
/// Labels are split on `.`, reversed, and rejoined. Empty labels are dropped.
/// Input is lowercased.
///
/// # Examples
///
/// - `example.com` → `com.example`
/// - `WWW.Example.COM` → `com.example.www`
pub fn to_reverse(domain: &str) -> String {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return String::new();
    }
    domain
        .split('.')
        .filter(|label| !label.is_empty())
        .rev()
        .collect::<Vec<_>>()
        .join(".")
}

/// Convert reverse notation (`com.example`) back to a normal domain (`example.com`).
pub fn from_reverse(rev: &str) -> String {
    let rev = rev.trim().trim_end_matches('.');
    if rev.is_empty() {
        return String::new();
    }
    rev.split('.')
        .filter(|label| !label.is_empty())
        .rev()
        .collect::<Vec<_>>()
        .join(".")
}

/// Safe filename stem from a reverse domain (replace characters that are
/// awkward on Windows/Unix filesystems).
pub fn reverse_to_filename(rev: &str) -> String {
    rev.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_basic() {
        assert_eq!(to_reverse("example.com"), "com.example");
        assert_eq!(to_reverse("en.wikipedia.org"), "org.wikipedia.en");
    }

    #[test]
    fn reverse_roundtrip() {
        let d = "blog.example.co.uk";
        assert_eq!(from_reverse(&to_reverse(d)), d);
    }

    #[test]
    fn reverse_case_and_trailing_dot() {
        assert_eq!(to_reverse("Example.COM."), "com.example");
    }

    #[test]
    fn reverse_empty() {
        assert_eq!(to_reverse(""), "");
        assert_eq!(from_reverse(""), "");
    }
}
