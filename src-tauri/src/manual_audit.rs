use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use url::Url;

const MAX_PAGES: usize = 50;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualAuditRequest {
    pub target_domain: String,
    #[serde(default)]
    pub brand: String,
    pub urls: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPageAudit {
    pub url: String,
    pub source_domain: String,
    pub status: String,
    pub http_status: Option<u16>,
    pub title: String,
    pub has_backlink: bool,
    pub backlink_count: usize,
    pub anchors: Vec<String>,
    pub rel_values: Vec<String>,
    pub has_brand_mention: bool,
    pub has_domain_mention: bool,
    pub note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualAuditReport {
    pub target_domain: String,
    pub audited: usize,
    pub confirmed_backlinks: usize,
    pub mentions_without_link: usize,
    pub blocked_or_skipped: usize,
    pub pages: Vec<ManualPageAudit>,
}

fn normalize_target(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .to_lowercase()
}

fn unsafe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

async fn validate_public_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Only HTTP(S) URLs are allowed");
    }
    let host = url.host_str().context("URL has no host")?;
    if host.eq_ignore_ascii_case("localhost") {
        bail!("Local addresses are not allowed");
    }
    let port = url.port_or_known_default().context("URL has no port")?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .context("DNS lookup failed")?;
    if addresses.into_iter().any(|address| unsafe_ip(address.ip())) {
        bail!("Private or local network addresses are not allowed");
    }
    Ok(())
}

fn robots_allows(body: &str, path: &str) -> bool {
    let mut applies = false;
    let mut disallow = Vec::new();
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "user-agent" => applies = value.trim() == "*",
            "disallow" if applies && !value.trim().is_empty() => {
                disallow.push(value.trim().to_string())
            }
            _ => {}
        }
    }
    !disallow.iter().any(|rule| path.starts_with(rule))
}

async fn audit_one(
    client: &reqwest::Client,
    raw: &str,
    target: &str,
    brand: &str,
) -> ManualPageAudit {
    let fallback = |status: &str, note: String| ManualPageAudit {
        url: raw.into(),
        source_domain: String::new(),
        status: status.into(),
        http_status: None,
        title: String::new(),
        has_backlink: false,
        backlink_count: 0,
        anchors: vec![],
        rel_values: vec![],
        has_brand_mention: false,
        has_domain_mention: false,
        note,
    };
    let url = match Url::parse(raw) {
        Ok(url) => url,
        Err(error) => return fallback("invalid", error.to_string()),
    };
    if let Err(error) = validate_public_url(&url).await {
        return fallback("skipped", error.to_string());
    }
    let source_domain = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_lowercase();
    if source_domain == target || source_domain.ends_with(&format!(".{target}")) {
        return fallback("skipped", "Target's own domain is excluded".into());
    }

    let robots_url = format!(
        "{}://{}/robots.txt",
        url.scheme(),
        url.host_str().unwrap_or_default()
    );
    if let Ok(response) = client.get(&robots_url).send().await {
        if response.status().is_success() {
            if let Ok(body) = response.text().await {
                if !robots_allows(&body, url.path()) {
                    return fallback("robots_blocked", "robots.txt disallows this path".into());
                }
            }
        }
    }
    tokio::time::sleep(Duration::from_millis(900)).await;
    let response = match client.get(url.clone()).send().await {
        Ok(r) => r,
        Err(error) => return fallback("request_failed", error.to_string()),
    };
    let status_code = response.status().as_u16();
    if status_code == 403 || status_code == 429 {
        let mut result = fallback(
            "blocked",
            format!("Server returned HTTP {status_code}; no bypass attempted"),
        );
        result.http_status = Some(status_code);
        result.source_domain = source_domain;
        return result;
    }
    if !response.status().is_success() {
        let mut result = fallback("http_error", format!("HTTP {status_code}"));
        result.http_status = Some(status_code);
        result.source_domain = source_domain;
        return result;
    }
    if response
        .content_length()
        .is_some_and(|size| size as usize > MAX_BODY_BYTES)
    {
        return fallback("too_large", "Page exceeds the 2 MB audit limit".into());
    }
    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(error) => return fallback("request_failed", error.to_string()),
    };
    if bytes.len() > MAX_BODY_BYTES {
        return fallback("too_large", "Page exceeds the 2 MB audit limit".into());
    }
    let body = String::from_utf8_lossy(&bytes);
    let document = Html::parse_document(&body);
    let anchor_selector = Selector::parse("a[href]").unwrap();
    let title_selector = Selector::parse("title").unwrap();
    let mut anchors = Vec::new();
    let mut rel_values = Vec::new();
    let mut count = 0;
    for element in document.select(&anchor_selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Ok(link) = url.join(href) else { continue };
        let host = link
            .host_str()
            .unwrap_or_default()
            .trim_start_matches("www.");
        if host == target || host.ends_with(&format!(".{target}")) {
            count += 1;
            let anchor = element
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            if !anchor.is_empty() && anchors.len() < 8 {
                anchors.push(anchor);
            }
            if let Some(rel) = element.value().attr("rel") {
                if !rel_values.iter().any(|v| v == rel) {
                    rel_values.push(rel.into());
                }
            }
        }
    }
    let visible_text = document
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    ManualPageAudit {
        url: url.into(),
        source_domain,
        status: "audited".into(),
        http_status: Some(status_code),
        title: document
            .select(&title_selector)
            .next()
            .map(|e| e.text().collect::<Vec<_>>().join(" ").trim().to_string())
            .unwrap_or_default(),
        has_backlink: count > 0,
        backlink_count: count,
        anchors,
        rel_values,
        has_brand_mention: !brand.is_empty() && visible_text.contains(&brand.to_lowercase()),
        has_domain_mention: visible_text.contains(target),
        note: "Fetched with robots.txt check and conservative limits".into(),
    }
}

pub async fn audit(request: ManualAuditRequest) -> Result<ManualAuditReport> {
    let target = normalize_target(&request.target_domain);
    if target.is_empty() || !target.contains('.') {
        bail!("A valid target domain is required");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("WebRadarSEOAudit/0.2 (+manual user-selected URL audit)")
        .build()?;
    let mut seen = HashSet::new();
    let urls = request
        .urls
        .into_iter()
        .filter(|url| seen.insert(url.trim().trim_end_matches('/').to_string()))
        .take(MAX_PAGES)
        .collect::<Vec<_>>();
    let mut pages = Vec::with_capacity(urls.len());
    for (index, url) in urls.iter().enumerate() {
        eprintln!(
            "[web-radar][manual-audit] page={}/{} url={}",
            index + 1,
            urls.len(),
            url
        );
        pages.push(audit_one(&client, url, &target, request.brand.trim()).await);
    }
    let confirmed_backlinks = pages.iter().filter(|page| page.has_backlink).count();
    let mentions_without_link = pages
        .iter()
        .filter(|page| !page.has_backlink && (page.has_brand_mention || page.has_domain_mention))
        .count();
    let blocked_or_skipped = pages.iter().filter(|page| page.status != "audited").count();
    Ok(ManualAuditReport {
        target_domain: target,
        audited: pages.len(),
        confirmed_backlinks,
        mentions_without_link,
        blocked_or_skipped,
        pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn respects_simple_robots_rules() {
        assert!(!robots_allows(
            "User-agent: *\nDisallow: /private",
            "/private/a"
        ));
        assert!(robots_allows(
            "User-agent: *\nDisallow: /private",
            "/public"
        ));
    }
    #[test]
    fn blocks_private_addresses() {
        assert!(unsafe_ip("127.0.0.1".parse().unwrap()));
        assert!(unsafe_ip("192.168.1.1".parse().unwrap()));
        assert!(!unsafe_ip("1.1.1.1".parse().unwrap()));
    }
}
