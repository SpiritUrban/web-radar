use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoDiscoveryRequest {
    pub provider: String,
    pub api_key: String,
    #[serde(default)]
    pub google_cx: String,
    pub targets: Vec<SeoTarget>,
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(default = "default_results_per_query")]
    pub results_per_query: u8,
}

fn default_country() -> String { "UA".into() }
fn default_results_per_query() -> u8 { 10 }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoTarget {
    pub domain: String,
    #[serde(default)]
    pub brand: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoQuery {
    pub kind: String,
    pub query: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoEvidence {
    pub target_domain: String,
    pub query_kind: String,
    pub query: String,
    pub title: String,
    pub url: String,
    pub source_domain: String,
    pub snippet: String,
    pub evidence_type: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoInsight {
    pub level: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeoDiscoveryReport {
    pub provider: String,
    pub generated_at: String,
    pub queries: Vec<SeoQuery>,
    pub evidence: Vec<SeoEvidence>,
    pub insights: Vec<SeoInsight>,
    pub unique_source_domains: usize,
    pub potential_backlinks: usize,
    pub brand_mentions: usize,
    pub reputation_mentions: usize,
}

#[derive(Debug, Deserialize)]
struct BraveEnvelope { web: Option<BraveWeb> }
#[derive(Debug, Deserialize)]
struct BraveWeb { #[serde(default)] results: Vec<BraveItem> }
#[derive(Debug, Deserialize)]
struct BraveItem { title: String, url: String, #[serde(default)] description: String }

#[derive(Debug, Deserialize)]
struct GoogleEnvelope { #[serde(default)] items: Vec<GoogleItem> }
#[derive(Debug, Deserialize)]
struct GoogleItem { title: String, link: String, #[serde(default)] snippet: String }

pub fn derive_brand(domain: &str) -> String {
    let host = domain.trim().trim_start_matches("https://").trim_start_matches("http://");
    let label = host.split('/').next().unwrap_or(host).trim_start_matches("www.").split('.').next().unwrap_or(host);
    label.split(['-', '_']).filter(|s| !s.is_empty()).map(|word| {
        let mut chars = word.chars();
        chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

pub fn build_queries(target: &SeoTarget) -> Vec<SeoQuery> {
    let domain = target.domain.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_lowercase();
    let brand = if target.brand.trim().is_empty() { derive_brand(&domain) } else { target.brand.trim().to_string() };
    let rows = [
        ("potential_backlink", format!("\"{domain}\" -site:{domain}"), "Pages mentioning the exact domain outside the target site"),
        ("brand_mention", format!("\"{brand}\" -site:{domain}"), "Unlinked and linked brand mentions"),
        ("url_mention", format!("\"https://{domain}\" -site:{domain}"), "Pages containing the canonical URL"),
        ("inpage_domain", format!("inpage:\"{domain}\" NOT site:{domain}"), "Domain text found in page content"),
        ("reputation", format!("\"{brand}\" (review OR reviews OR відгуки OR скарги OR scam) -site:{domain}"), "Reviews, complaints and reputation signals"),
        ("documents", format!("\"{domain}\" (filetype:pdf OR filetype:doc OR filetype:xls) -site:{domain}"), "Documents and citations mentioning the domain"),
    ];
    rows.into_iter().map(|(kind, query, purpose)| SeoQuery { kind: kind.into(), query, purpose: purpose.into() }).collect()
}

fn source_domain(raw: &str) -> String {
    Url::parse(raw).ok().and_then(|url| url.host_str().map(|h| h.trim_start_matches("www.").to_lowercase())).unwrap_or_default()
}

fn classify(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "potential_backlink" | "url_mention" | "inpage_domain" => ("potential_backlink", "medium"),
        "reputation" => ("reputation_mention", "medium"),
        "documents" => ("document_citation", "medium"),
        _ => ("brand_mention", "low"),
    }
}

pub async fn discover(request: SeoDiscoveryRequest) -> Result<SeoDiscoveryReport> {
    if request.api_key.trim().is_empty() { bail!("API key is required"); }
    if request.targets.is_empty() { bail!("At least one target is required"); }
    if request.provider == "google" && request.google_cx.trim().is_empty() { bail!("Google Search Engine ID (cx) is required"); }
    if request.provider != "brave" && request.provider != "google" { bail!("Unsupported provider: {}", request.provider); }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent("WebRadar/0.2 SEO discovery")
        .build()?;
    let limit = request.results_per_query.clamp(1, 20);
    let mut all_queries = Vec::new();
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();

    for target in &request.targets {
        for query in build_queries(target) {
            eprintln!("[web-radar][seo] provider={} target={} kind={} query={}", request.provider, target.domain, query.kind, query.query);
            let items: Vec<(String, String, String)> = if request.provider == "brave" {
                let response = client.get("https://api.search.brave.com/res/v1/web/search")
                    .header("Accept", "application/json")
                    .header("X-Subscription-Token", request.api_key.trim())
                    .query(&[("q", query.query.as_str()), ("count", &limit.to_string()), ("country", request.country.as_str()), ("safesearch", "moderate")])
                    .send().await.context("Brave Search request failed")?
                    .error_for_status().context("Brave Search returned an error")?
                    .json::<BraveEnvelope>().await.context("Invalid Brave Search response")?;
                response.web.map(|w| w.results).unwrap_or_default().into_iter().map(|x| (x.title, x.url, x.description)).collect()
            } else {
                let response = client.get("https://www.googleapis.com/customsearch/v1")
                    .query(&[("key", request.api_key.trim()), ("cx", request.google_cx.trim()), ("q", query.query.as_str()), ("num", &limit.min(10).to_string())])
                    .send().await.context("Google Custom Search request failed")?
                    .error_for_status().context("Google Custom Search returned an error")?
                    .json::<GoogleEnvelope>().await.context("Invalid Google Custom Search response")?;
                response.items.into_iter().map(|x| (x.title, x.link, x.snippet)).collect()
            };

            for (title, url, snippet) in items {
                let host = source_domain(&url);
                let target_host = target.domain.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').trim_start_matches("www.");
                if host.is_empty() || host == target_host || host.ends_with(&format!(".{target_host}")) { continue; }
                let dedupe = format!("{}|{}", target_host, url.trim_end_matches('/'));
                if !seen.insert(dedupe) { continue; }
                let (evidence_type, confidence) = classify(&query.kind);
                evidence.push(SeoEvidence {
                    target_domain: target_host.into(), query_kind: query.kind.clone(), query: query.query.clone(),
                    title, url, source_domain: host, snippet, evidence_type: evidence_type.into(), confidence: confidence.into(),
                });
            }
            all_queries.push(query);
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    let unique_source_domains = evidence.iter().map(|e| e.source_domain.as_str()).collect::<HashSet<_>>().len();
    let potential_backlinks = evidence.iter().filter(|e| e.evidence_type == "potential_backlink").count();
    let brand_mentions = evidence.iter().filter(|e| e.evidence_type == "brand_mention").count();
    let reputation_mentions = evidence.iter().filter(|e| e.evidence_type == "reputation_mention").count();
    let mut by_domain = BTreeMap::<String, usize>::new();
    for item in &evidence { *by_domain.entry(item.source_domain.clone()).or_default() += 1; }
    let strongest = by_domain.iter().max_by_key(|(_, count)| *count).map(|(d, c)| format!("{d} ({c})"));
    let mut insights = vec![SeoInsight {
        level: "info".into(), title: "Discovery, not link verification".into(),
        detail: "Search results prove index visibility or a mention. Open the source page before treating it as a confirmed live backlink.".into(),
    }];
    if potential_backlinks > 0 { insights.push(SeoInsight { level: "positive".into(), title: "Backlink prospects found".into(), detail: format!("{potential_backlinks} external pages are candidates for manual or automated link verification.") }); }
    if reputation_mentions > 0 { insights.push(SeoInsight { level: "warning".into(), title: "Reputation results need review".into(), detail: format!("{reputation_mentions} review/complaint-oriented results were found; sentiment is not inferred from snippets alone.") }); }
    if let Some(strongest) = strongest { insights.push(SeoInsight { level: "info".into(), title: "Most visible source".into(), detail: format!("The most frequently discovered external domain is {strongest}.") }); }

    Ok(SeoDiscoveryReport {
        provider: request.provider, generated_at: chrono::Utc::now().to_rfc3339(), queries: all_queries,
        evidence, insights, unique_source_domains, potential_backlinks, brand_mentions, reputation_mentions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_safe_discovery_queries() {
        let queries = build_queries(&SeoTarget { domain: "my-transfer.com.ua".into(), brand: "My Transfer".into() });
        assert!(queries.iter().all(|q| q.query.contains("-site:my-transfer.com.ua") || q.query.contains("NOT site:my-transfer.com.ua")));
        assert!(queries.iter().any(|q| q.query == "\"My Transfer\" -site:my-transfer.com.ua"));
        assert!(queries.iter().any(|q| q.kind == "reputation"));
    }

    #[test]
    fn derives_human_brand_from_domain() {
        assert_eq!(derive_brand("https://my-transfer.com.ua/"), "My Transfer");
    }
}
