//! Result shapes shared by the streaming scan, the indexed query engine,
//! the CLI and the desktop UI.

use serde::{Deserialize, Serialize};

/// One linked domain with its rank (inbound source or outbound destination).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkEntry {
    /// Domain in normal form (`other.com`).
    pub domain: String,
    /// Selected rank metric of that domain (0.0 when the graph has no rank row).
    pub rank: f64,
    /// Position in the global ranking, 1 = strongest. `None` without a rank index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
}

impl LinkEntry {
    pub fn new(domain: String, rank: f64) -> Self {
        Self {
            domain,
            rank,
            position: None,
        }
    }
}

/// Full report for one target domain, written as `results/{reversed-domain}.json`.
///
/// Field order and names are kept backwards compatible with 0.2 result files;
/// everything added since is `skip_serializing_if`-optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetResult {
    /// Target domain in normal form (`example.com`).
    pub domain: String,
    /// Whether the domain exists in the Common Crawl vertices graph.
    pub found: bool,
    /// Rank of the target itself (PageRank or Harmonic). `null` if not found.
    pub rank: Option<f64>,
    /// Domains that link *to* this target, sorted by rank descending.
    pub inbound: Vec<LinkEntry>,
    /// Domains this target links *to*, sorted by rank descending.
    pub outbound: Vec<LinkEntry>,

    /// Which metric `rank` holds — `pagerank` or `harmonic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// Node id inside the Common Crawl graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<u32>,
    /// Position of the target in the global ranking, 1 = strongest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    /// Total inbound domains found, before any display limit was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbound_total: Option<usize>,
    /// Total outbound domains found, before any display limit was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_total: Option<usize>,
    /// How the answer was produced: `index` or `scan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl TargetResult {
    pub fn not_found(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            found: false,
            rank: None,
            inbound: Vec::new(),
            outbound: Vec::new(),
            metric: None,
            node_id: None,
            position: None,
            inbound_total: None,
            outbound_total: None,
            source: None,
        }
    }

    /// `domain,direction,rank,position` — one row per linked domain.
    pub fn to_csv(&self) -> String {
        let mut out = String::from("domain,direction,rank,position\n");
        for (direction, list) in [("inbound", &self.inbound), ("outbound", &self.outbound)] {
            for entry in list {
                out.push_str(&format!(
                    "{},{},{},{}\n",
                    entry.domain,
                    direction,
                    entry.rank,
                    entry.position.map(|p| p.to_string()).unwrap_or_default()
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_has_a_row_per_link_in_both_directions() {
        let mut result = TargetResult::not_found("example.com");
        result.inbound.push(LinkEntry::new("a.com".into(), 1.0));
        result.outbound.push(LinkEntry::new("b.com".into(), 2.0));
        let csv = result.to_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header + two links: {csv}");
        assert!(lines[1].starts_with("a.com,inbound,"));
        assert!(lines[2].starts_with("b.com,outbound,"));
    }

    #[test]
    fn legacy_result_files_still_deserialize() {
        let legacy = r#"{"domain":"a.com","found":true,"rank":1.0,"inbound":[{"domain":"b.com","rank":2.0}],"outbound":[]}"#;
        let parsed: TargetResult = serde_json::from_str(legacy).expect("parse 0.2 result file");
        assert_eq!(parsed.inbound[0].domain, "b.com");
        assert_eq!(parsed.position, None);

        // …and the fields added since are camelCase, like every other payload.
        let mut written = parsed;
        written.inbound_total = Some(1);
        let json = serde_json::to_string(&written).expect("serialize");
        assert!(
            json.contains("\"inboundTotal\":1"),
            "unexpected shape: {json}"
        );
    }
}
