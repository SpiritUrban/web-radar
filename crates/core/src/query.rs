//! Answering questions about one domain, using whatever index tiers exist.
//!
//! The engine degrades honestly: every answer carries [`DomainReport::warnings`]
//! saying what was *not* available, so the UI can offer to build the missing
//! tier instead of quietly returning half a picture.

use std::path::Path;
use std::time::Instant;

use ahash::AHashMap;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::RankMetric;
use crate::index::{
    EdgeIndex, GraphIndex, GraphSources, InboundIndex, RankIndex, Tier, VertexIndex,
};
use crate::model::{LinkEntry, TargetResult};
use crate::reverse::{from_reverse, normalize_domain, to_reverse};

/// Default cap on how many neighbours one query materialises.
pub const DEFAULT_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOptions {
    pub include_inbound: bool,
    pub include_outbound: bool,
    /// Max neighbours per direction.
    pub limit: usize,
    pub metric: RankMetric,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            include_inbound: true,
            include_outbound: true,
            limit: DEFAULT_LIMIT,
            metric: RankMetric::Pagerank,
        }
    }
}

/// What the currently built tiers let the engine answer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub lookup: bool,
    pub outbound: bool,
    pub inbound: bool,
    pub ranks: bool,
}

impl Capabilities {
    pub fn can_query(&self) -> bool {
        self.lookup
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReport {
    pub domain: String,
    pub reverse_domain: String,
    pub found: bool,
    pub node_id: Option<u32>,
    pub metric: String,
    pub rank: Option<f64>,
    pub position: Option<u32>,
    pub inbound: Vec<LinkEntry>,
    pub outbound: Vec<LinkEntry>,
    /// Real totals, which can exceed the returned list length.
    pub inbound_total: u64,
    pub outbound_total: u64,
    pub inbound_truncated: bool,
    pub outbound_truncated: bool,
    pub elapsed_ms: u64,
    pub warnings: Vec<String>,
}

impl DomainReport {
    /// The 0.2-compatible on-disk shape, for `results/*.json`.
    pub fn to_target_result(&self) -> TargetResult {
        TargetResult {
            domain: self.domain.clone(),
            found: self.found,
            rank: self.rank,
            inbound: self.inbound.clone(),
            outbound: self.outbound.clone(),
            metric: Some(self.metric.clone()),
            node_id: self.node_id,
            position: self.position,
            inbound_total: Some(self.inbound_total as usize),
            outbound_total: Some(self.outbound_total as usize),
            source: Some("index".into()),
        }
    }
}

/// Read-only handle over the source files plus every index tier that exists.
pub struct Engine {
    vertices: Option<VertexIndex>,
    edges: Option<EdgeIndex>,
    ranks: Option<RankIndex>,
    inbound: Option<InboundIndex>,
    capabilities: Capabilities,
}

impl Engine {
    /// Open whatever is available. Missing tiers are not an error — they are
    /// reported through [`Engine::capabilities`].
    pub fn open(index: &GraphIndex, sources: &GraphSources) -> Result<Self> {
        let status = index.status(sources);
        let mut engine = Self {
            vertices: None,
            edges: None,
            ranks: None,
            inbound: None,
            capabilities: Capabilities::default(),
        };

        if status.is_ready(Tier::Lookup) {
            engine.vertices = open_optional(
                "vertices",
                VertexIndex::open(&index.vertices_path(), &sources.vertices),
            );
            engine.edges = open_optional(
                "edges",
                EdgeIndex::open(&index.edges_path(), &sources.edges),
            );
        }
        if status.is_ready(Tier::Ranks) {
            engine.ranks = open_optional("ranks", RankIndex::open(&index.ranks_path()));
        }
        if status.is_ready(Tier::Inbound) {
            engine.inbound = open_optional(
                "inbound",
                InboundIndex::open(&index.inbound_offsets_path(), &index.inbound_sources_path()),
            );
        }

        engine.capabilities = Capabilities {
            lookup: engine
                .vertices
                .as_ref()
                .is_some_and(|index| index.sorted_by_name),
            outbound: engine
                .edges
                .as_ref()
                .is_some_and(|index| index.sorted_by_from),
            inbound: engine.inbound.is_some(),
            ranks: engine.ranks.is_some(),
        };
        Ok(engine)
    }

    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Total domains in the graph, when known.
    pub fn node_count(&self) -> u64 {
        self.vertices
            .as_ref()
            .map(|index| index.node_count)
            .unwrap_or(0)
    }

    /// Node id of a domain given in any form (`https://www.Example.com/x`).
    pub fn resolve(&self, domain: &str) -> Result<Option<u32>> {
        let Some(vertices) = self.vertices.as_ref() else {
            bail!("індекс пошуку не побудований");
        };
        let reverse = to_reverse(domain);
        if reverse.is_empty() {
            bail!("«{domain}» не схоже на доменне ім'я");
        }
        vertices.id_of(&reverse)
    }

    /// The full picture for one domain.
    pub fn query(&self, domain: &str, options: QueryOptions) -> Result<DomainReport> {
        let started = Instant::now();
        let clean = normalize_domain(domain);
        let reverse = to_reverse(&clean);
        let mut report = DomainReport {
            domain: clean.clone(),
            reverse_domain: reverse.clone(),
            found: false,
            node_id: None,
            metric: options.metric.as_str().to_string(),
            rank: None,
            position: None,
            inbound: Vec::new(),
            outbound: Vec::new(),
            inbound_total: 0,
            outbound_total: 0,
            inbound_truncated: false,
            outbound_truncated: false,
            elapsed_ms: 0,
            warnings: Vec::new(),
        };
        if reverse.is_empty() {
            bail!("«{domain}» не схоже на доменне ім'я");
        }
        let Some(vertices) = self.vertices.as_ref() else {
            bail!(
                "індекс пошуку не побудований — відкрийте вкладку «Дані» та побудуйте рівень «{}»",
                Tier::Lookup.label()
            );
        };

        let Some(node_id) = vertices.id_of(&reverse)? else {
            report.elapsed_ms = started.elapsed().as_millis() as u64;
            report.warnings.push(format!(
                "Домен {clean} відсутній у цьому випуску Common Crawl. Перевірте написання або візьміть свіжіший граф."
            ));
            return Ok(report);
        };
        report.found = true;
        report.node_id = Some(node_id);

        if let Some(ranks) = self.ranks.as_ref() {
            if let Some(row) = ranks.get(node_id) {
                report.rank = Some(row.value(options.metric));
                report.position = row.position(options.metric);
            }
        } else {
            report.warnings.push(format!(
                "Рівень «{}» не побудований — рейтинги показані як 0.",
                Tier::Ranks.label()
            ));
        }

        // --- neighbours ---
        let mut inbound_ids: Vec<u32> = Vec::new();
        if options.include_inbound {
            match self.inbound.as_ref() {
                Some(index) => {
                    let (ids, total) = index.sources_of(node_id, options.limit);
                    report.inbound_total = total;
                    report.inbound_truncated = total > ids.len() as u64;
                    inbound_ids = ids;
                }
                None => report.warnings.push(format!(
                    "Рівень «{}» не побудований — хто посилається на домен, поки невідомо.",
                    Tier::Inbound.label()
                )),
            }
        }

        let mut outbound_ids: Vec<u32> = Vec::new();
        if options.include_outbound {
            match self.edges.as_ref() {
                Some(index) if self.capabilities.outbound => {
                    let (ids, truncated) = index.outbound(node_id, options.limit)?;
                    report.outbound_total = ids.len() as u64;
                    report.outbound_truncated = truncated;
                    outbound_ids = ids;
                }
                _ => report.warnings.push(
                    "Файл edges не відсортований за from_id — вихідні зв'язки недоступні.".into(),
                ),
            }
        }

        // One name resolution pass for both directions.
        let mut all_ids = Vec::with_capacity(inbound_ids.len() + outbound_ids.len());
        all_ids.extend_from_slice(&inbound_ids);
        all_ids.extend_from_slice(&outbound_ids);
        let mut names: AHashMap<u32, String> = AHashMap::with_capacity(all_ids.len());
        vertices.names_of(&all_ids, |id, name| {
            names.insert(id, from_reverse(name));
        })?;

        report.inbound = self.entries(&inbound_ids, &names, options.metric);
        report.outbound = self.entries(&outbound_ids, &names, options.metric);
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    fn entries(
        &self,
        ids: &[u32],
        names: &AHashMap<u32, String>,
        metric: RankMetric,
    ) -> Vec<LinkEntry> {
        let mut entries: Vec<LinkEntry> = ids
            .iter()
            .filter_map(|id| {
                let domain = names.get(id)?.clone();
                let row = self.ranks.as_ref().and_then(|ranks| ranks.get(*id));
                Some(LinkEntry {
                    domain,
                    rank: row.map(|row| row.value(metric)).unwrap_or(0.0),
                    position: row.and_then(|row| row.position(metric)),
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.domain.cmp(&b.domain))
        });
        entries
    }

    /// Write a report next to the other results, keeping the 0.2 file shape.
    pub fn write_result(
        &self,
        results_dir: &Path,
        report: &DomainReport,
    ) -> Result<std::path::PathBuf> {
        std::fs::create_dir_all(results_dir)
            .with_context(|| format!("не вдалося створити {}", results_dir.display()))?;
        let name = crate::reverse::reverse_to_filename(&report.reverse_domain) + ".json";
        let path = results_dir.join(name);
        let json = serde_json::to_string_pretty(&report.to_target_result())?;
        std::fs::write(&path, json + "\n")
            .with_context(|| format!("не вдалося записати {}", path.display()))?;
        Ok(path)
    }
}

fn open_optional<T>(what: &str, opened: Result<T>) -> Option<T> {
    match opened {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("index tier {what} is present but unusable: {error:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::GraphIndex;
    use crate::progress::Progress;

    /// Ranks are stored as `f32` (halving a 2 GB index), so values come back
    /// with ~7 significant digits rather than bit-identical.
    fn assert_close(got: Option<f64>, want: f64) {
        let got = got.expect("a rank was expected");
        assert!(
            (got - want).abs() <= want.abs() * 1e-6,
            "rank {got:e} is not within f32 precision of {want:e}"
        );
    }

    /// A three-domain graph where every relationship is known by hand.
    fn fixture(dir: &Path) -> (GraphIndex, GraphSources) {
        // ids: 0 com.aaa, 1 com.bbb, 2 org.ccc  →  aaa.com, bbb.com, ccc.org
        std::fs::write(
            dir.join("vertices.txt"),
            "0\tcom.aaa\t1\n1\tcom.bbb\t1\n2\torg.ccc\t1\n",
        )
        .expect("vertices");
        // bbb.com and ccc.org both link to aaa.com; aaa.com links to ccc.org.
        std::fs::write(dir.join("edges.txt"), "0\t2\n1\t0\n2\t0\n").expect("edges");
        std::fs::write(
            dir.join("ranks.txt"),
            "#harmonicc_pos\t#harmonicc_val\t#pr_pos\t#pr_val\t#host_rev\t#n_hosts\n\
             1\t900.0\t1\t5.0E-9\tcom.aaa\t1\n\
             2\t800.0\t3\t3.0E-9\tcom.bbb\t1\n\
             3\t700.0\t2\t4.0E-9\torg.ccc\t1\n",
        )
        .expect("ranks");

        let sources = GraphSources {
            vertices: dir.join("vertices.txt"),
            edges: dir.join("edges.txt"),
            ranks: dir.join("ranks.txt"),
        };
        let index = GraphIndex::new(dir.join("index"));
        (index, sources)
    }

    #[test]
    fn answers_both_directions_with_ranks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (index, sources) = fixture(dir.path());
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build");
        let engine = Engine::open(&index, &sources).expect("open");

        let capabilities = engine.capabilities();
        assert!(
            capabilities.lookup
                && capabilities.outbound
                && capabilities.inbound
                && capabilities.ranks
        );

        let report = engine
            .query("https://www.AAA.com/some/path?x=1", QueryOptions::default())
            .expect("query");
        assert!(report.found, "aaa.com must be found: {:?}", report.warnings);
        assert_eq!(report.domain, "aaa.com");
        assert_eq!(report.node_id, Some(0));
        assert_close(report.rank, 5.0e-9);
        assert_eq!(report.position, Some(1));

        let inbound: Vec<&str> = report.inbound.iter().map(|e| e.domain.as_str()).collect();
        assert_eq!(
            inbound,
            vec!["ccc.org", "bbb.com"],
            "sorted by rank descending"
        );
        assert_eq!(report.inbound_total, 2);
        let outbound: Vec<&str> = report.outbound.iter().map(|e| e.domain.as_str()).collect();
        assert_eq!(outbound, vec!["ccc.org"]);
        assert!(
            report.warnings.is_empty(),
            "unexpected warnings: {:?}",
            report.warnings
        );
    }

    #[test]
    fn missing_domain_is_reported_not_invented() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (index, sources) = fixture(dir.path());
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build");
        let engine = Engine::open(&index, &sources).expect("open");

        let report = engine
            .query("nowhere.example", QueryOptions::default())
            .expect("query");
        assert!(!report.found);
        assert!(report.inbound.is_empty() && report.outbound.is_empty());
        assert!(
            report.warnings.iter().any(|w| w.contains("відсутній")),
            "the user must be told why: {:?}",
            report.warnings
        );
    }

    #[test]
    fn without_the_inbound_tier_it_says_so_instead_of_answering_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (index, sources) = fixture(dir.path());
        index
            .build(&sources, &[Tier::Lookup, Tier::Ranks], &Progress::silent())
            .expect("build");
        let engine = Engine::open(&index, &sources).expect("open");
        assert!(!engine.capabilities().inbound);

        let report = engine
            .query("aaa.com", QueryOptions::default())
            .expect("query");
        assert!(report.found);
        assert!(report.inbound.is_empty());
        assert_eq!(report.inbound_total, 0);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("Зворотні посилання")),
            "a missing tier must be named: {:?}",
            report.warnings
        );
        // Outbound and ranks still work.
        assert_eq!(report.outbound.len(), 1);
        assert_close(report.rank, 5.0e-9);
    }

    #[test]
    fn harmonic_metric_changes_both_value_and_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (index, sources) = fixture(dir.path());
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build");
        let engine = Engine::open(&index, &sources).expect("open");

        let options = QueryOptions {
            metric: RankMetric::Harmonic,
            ..QueryOptions::default()
        };
        let report = engine.query("aaa.com", options).expect("query");
        assert_eq!(report.rank, Some(900.0));
        assert_eq!(report.position, Some(1));
        let inbound: Vec<&str> = report.inbound.iter().map(|e| e.domain.as_str()).collect();
        // By harmonic, bbb.com (800) outranks ccc.org (700) — the reverse of PageRank.
        assert_eq!(inbound, vec!["bbb.com", "ccc.org"]);
    }

    #[test]
    fn writes_a_result_file_readable_by_the_previous_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (index, sources) = fixture(dir.path());
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build");
        let engine = Engine::open(&index, &sources).expect("open");
        let report = engine
            .query("aaa.com", QueryOptions::default())
            .expect("query");

        let results = dir.path().join("results");
        let path = engine.write_result(&results, &report).expect("write");
        assert_eq!(path.file_name().unwrap(), "com.aaa.json");

        let raw = std::fs::read_to_string(&path).expect("read back");
        let parsed: TargetResult = serde_json::from_str(&raw).expect("parse");
        assert!(parsed.found);
        assert_eq!(parsed.inbound.len(), 2);
        assert_eq!(parsed.metric.as_deref(), Some("pagerank"));
    }
}
