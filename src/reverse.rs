//! Reverse domain name notation helpers.
//!
//! Common Crawl stores domains as `com.example` instead of `example.com`.

/// Strip a messy user input down to a bare hostname.
///
/// Accepts things JS devs paste from the browser:
/// - `https://www.Example.com/path?q=1` → `example.com`
/// - `example.com.` → `example.com`
/// - `//cdn.example.com` → `cdn.example.com`
pub fn normalize_domain(input: &str) -> String {
    let mut s = input.trim().to_ascii_lowercase();

    // Drop surrounding quotes if someone copy-pasted from JSON.
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s = s[1..s.len() - 1].trim().to_string();
    }

    // scheme://
    if let Some(pos) = s.find("://") {
        s = s[pos + 3..].to_string();
    } else if let Some(stripped) = s.strip_prefix("//") {
        s = stripped.to_string();
    }

    // drop path / query / fragment
    if let Some(pos) = s.find(['/', '?', '#', '\\']) {
        s = s[..pos].to_string();
    }

    // drop userinfo user:pass@host
    if let Some(pos) = s.rfind('@') {
        s = s[pos + 1..].to_string();
    }

    // drop port host:443
    if let Some(pos) = s.rfind(':') {
        // avoid mangling IPv6; we only care about domain names here
        if s.bytes().filter(|&b| b == b':').count() == 1 {
            s = s[..pos].to_string();
        }
    }

    // trailing dots
    while s.ends_with('.') {
        s.pop();
    }

    // optional leading www.
    if let Some(stripped) = s.strip_prefix("www.") {
        s = stripped.to_string();
    }

    s
}

/// Convert a normal domain (`example.com`) to reverse notation (`com.example`).
///
/// Input may be a full URL — it is normalized first.
pub fn to_reverse(domain: &str) -> String {
    let domain = normalize_domain(domain);
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

/// Safe filename stem from a reverse domain.
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
    fn normalize_urls() {
        assert_eq!(
            normalize_domain("https://www.Example.com/path?x=1"),
            "example.com"
        );
        assert_eq!(
            normalize_domain("https://my-transfer.com.ua/"),
            "my-transfer.com.ua"
        );
        assert_eq!(normalize_domain("example.com:443"), "example.com");
        assert_eq!(normalize_domain("  EXAMPLE.COM.  "), "example.com");
    }

    #[test]
    fn reverse_basic() {
        assert_eq!(to_reverse("example.com"), "com.example");
        assert_eq!(to_reverse("en.wikipedia.org"), "org.wikipedia.en");
        assert_eq!(
            to_reverse("https://my-transfer.com.ua/foo"),
            "ua.com.my-transfer"
        );
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
