//! The index that makes a 79 GB graph answerable in milliseconds.
//!
//! Three tiers, each independently buildable and droppable:
//!
//! | tier | what it enables | build | size (2026 apr–jun graph) |
//! |---|---|---|---|
//! | [`Tier::Lookup`] | find a domain, list where it links **to** | one pass over vertices + sampling of edges | ~20 MB |
//! | [`Tier::Ranks`] | PageRank / harmonic for every domain shown | one pass over ranks + a join | ~1.9 GB |
//! | [`Tier::Inbound`] | who links **to** a domain — the backlink question | one pass over edges + external sort | ~16 GB |
//!
//! Without any of them the app still works through [`crate::scan`], which reads
//! all three files on every run. That is the honest fallback, not the plan.

pub mod edges;
pub mod inbound;
pub mod io;
pub mod ranks;
pub mod vertices;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::data_source::{crawl_from_path, setup_guide, GraphFile, SetupGuide, DEFAULT_CRAWL};
use crate::progress::Progress;

pub use edges::EdgeIndex;
pub use inbound::InboundIndex;
pub use ranks::{RankIndex, RankRow};
pub use vertices::VertexIndex;

/// Bumped whenever an on-disk layout changes; older indexes are rebuilt, not
/// misread.
pub const FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// The three Common Crawl files an index is built from.
#[derive(Debug, Clone)]
pub struct GraphSources {
    pub vertices: PathBuf,
    pub edges: PathBuf,
    pub ranks: PathBuf,
}

/// Identity of a source file, so a stale index can be spotted before it lies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFingerprint {
    pub path: String,
    pub size: u64,
    pub modified: Option<i64>,
}

impl SourceFingerprint {
    pub fn of(path: &Path) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            path: path.to_string_lossy().into_owned(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        }
    }
}

/// What the UI shows for each source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub kind: String,
    pub path: String,
    pub size_bytes: u64,
    pub exists: bool,
    /// Whether the file on disk is still gzipped.
    pub compressed: bool,
    /// Whether being gzipped is a problem for this particular file: queries
    /// seek into `vertices` and `edges`, but `ranks` is only ever streamed.
    pub must_be_unpacked: bool,
}

impl GraphSources {
    pub fn list(&self) -> [(&'static str, &Path); 3] {
        [
            ("vertices", self.vertices.as_path()),
            ("edges", self.edges.as_path()),
            ("ranks", self.ranks.as_path()),
        ]
    }

    pub fn status(&self) -> Vec<SourceStatus> {
        self.list()
            .into_iter()
            .map(|(kind, path)| SourceStatus {
                kind: kind.to_string(),
                path: path.to_string_lossy().into_owned(),
                size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
                exists: path.is_file(),
                compressed: is_compressed(path),
                // Only files that get seeked into have to be unpacked.
                must_be_unpacked: GraphFile::parse(kind).is_some_and(GraphFile::must_be_unpacked),
            })
            .collect()
    }

    fn size_of(&self, path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    /// Uncompressed byte size to reason about.
    ///
    /// A missing or still-gzipped file would otherwise make every estimate
    /// read "≈0 Б", which tells the user nothing at the exact moment they are
    /// deciding whether they have room for this.
    fn text_size_of(&self, path: &Path, file: GraphFile) -> u64 {
        let actual = self.size_of(path);
        if actual == 0 || is_compressed(path) {
            file.unpacked_bytes()
        } else {
            actual
        }
    }

    /// Rough node count before anything is built: vertices lines average ~28 B.
    fn estimated_nodes(&self) -> u64 {
        (self.text_size_of(&self.vertices, GraphFile::Vertices) / 28).max(1)
    }

    /// Rough edge count before anything is built: edges lines average ~18 B.
    fn estimated_edges(&self) -> u64 {
        (self.text_size_of(&self.edges, GraphFile::Edges) / 18).max(1)
    }

    /// The crawl these paths belong to, falling back to the reference release.
    pub fn crawl(&self) -> String {
        crawl_from_path(&self.edges)
            .or_else(|| crawl_from_path(&self.vertices))
            .or_else(|| crawl_from_path(&self.ranks))
            .unwrap_or_else(|| DEFAULT_CRAWL.to_string())
    }

    /// Directory the files are expected in — what the setup text points at.
    pub fn expected_dir(&self) -> PathBuf {
        self.vertices
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn is_compressed(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gz"))
}

// ---------------------------------------------------------------------------
// Tiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Lookup,
    Ranks,
    Inbound,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Lookup, Tier::Ranks, Tier::Inbound];

    pub fn key(self) -> &'static str {
        match self {
            Tier::Lookup => "lookup",
            Tier::Ranks => "ranks",
            Tier::Inbound => "inbound",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Tier::ALL.into_iter().find(|tier| tier.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Lookup => "Пошук доменів",
            Tier::Ranks => "Рейтинги",
            Tier::Inbound => "Зворотні посилання",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tier::Lookup => "Миттєво знаходить домен у графі та показує, куди він посилається.",
            Tier::Ranks => "Додає PageRank і Harmonic до кожного домену у результатах.",
            Tier::Inbound => {
                "Головне: хто посилається на домен. Без нього кожен запит читає весь файл edges."
            }
        }
    }

    fn files(self, root: &Path) -> Vec<PathBuf> {
        match self {
            Tier::Lookup => vec![root.join("vertices.idx"), root.join("edges.idx")],
            Tier::Ranks => vec![root.join("ranks.bin")],
            Tier::Inbound => vec![root.join("inbound.off"), root.join("inbound.src")],
        }
    }

    /// Which source files this tier's answers depend on.
    fn sources(self, sources: &GraphSources) -> Vec<SourceFingerprint> {
        match self {
            Tier::Lookup => vec![
                SourceFingerprint::of(&sources.vertices),
                SourceFingerprint::of(&sources.edges),
            ],
            Tier::Ranks => vec![
                SourceFingerprint::of(&sources.vertices),
                SourceFingerprint::of(&sources.ranks),
            ],
            Tier::Inbound => vec![
                SourceFingerprint::of(&sources.vertices),
                SourceFingerprint::of(&sources.edges),
            ],
        }
    }

    /// Bytes the finished tier occupies, estimated from the source sizes.
    pub fn estimated_bytes(self, sources: &GraphSources) -> u64 {
        let nodes = sources.estimated_nodes();
        let edges = sources.estimated_edges();
        match self {
            // ~20 B per block entry, one block per 256 domains, plus the edge samples.
            Tier::Lookup => {
                nodes / u64::from(vertices::BLOCK_LINES) * 40
                    + sources.size_of(&sources.edges) / edges::BLOCK_BYTES * 16
            }
            Tier::Ranks => nodes * ranks::ROW_BYTES as u64 + 64,
            Tier::Inbound => inbound::estimate_bytes(edges, nodes),
        }
    }

    /// Peak temporary space the build needs on top of the result.
    pub fn estimated_temp_bytes(self, sources: &GraphSources) -> u64 {
        match self {
            Tier::Lookup => 0,
            // One record per rank row: 1 B length + ~18 B name + 16 B values.
            Tier::Ranks => sources.estimated_nodes() * 35,
            Tier::Inbound => inbound::estimate_temp_bytes(sources.estimated_edges()),
        }
    }

    /// Relative build cost, in bytes touched — used to weight the progress bar.
    fn work_units(self, sources: &GraphSources) -> u64 {
        let vertices = sources.size_of(&sources.vertices);
        let edges = sources.size_of(&sources.edges);
        let ranks = sources.size_of(&sources.ranks);
        match self {
            Tier::Lookup => vertices + edges / 512,
            Tier::Ranks => ranks + vertices + self.estimated_temp_bytes(sources) * 2,
            Tier::Inbound => edges + self.estimated_temp_bytes(sources) * 2,
        }
        .max(1)
    }
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierRecord {
    pub built_at: String,
    pub built_by: String,
    pub bytes: u64,
    pub sources: Vec<SourceFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexMeta {
    pub format: u32,
    #[serde(default)]
    pub node_count: u64,
    /// `max id + 1` — the length of every id-addressed array.
    #[serde(default)]
    pub node_capacity: u64,
    #[serde(default)]
    pub edge_count: u64,
    #[serde(default)]
    pub tiers: BTreeMap<String, TierRecord>,
}

impl Default for IndexMeta {
    fn default() -> Self {
        Self {
            format: FORMAT_VERSION,
            node_count: 0,
            node_capacity: 0,
            edge_count: 0,
            tiers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TierState {
    /// Never built, or its files were removed.
    Missing,
    /// Built and matching the current source files.
    Ready,
    /// Built, but a source file changed since — answers would be wrong.
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierStatus {
    pub key: String,
    pub label: String,
    pub description: String,
    pub state: TierState,
    pub bytes: u64,
    pub estimated_bytes: u64,
    pub estimated_temp_bytes: u64,
    pub built_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub root: String,
    pub node_count: u64,
    pub edge_count: u64,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub tiers: Vec<TierStatus>,
    pub sources: Vec<SourceStatus>,
    /// Everything the index needs but does not have, in plain language.
    pub blockers: Vec<String>,
    /// Names, links and sizes for getting the data — rendered when files are missing.
    pub setup: SetupGuide,
}

impl IndexStatus {
    pub fn tier(&self, tier: Tier) -> Option<&TierStatus> {
        self.tiers.iter().find(|status| status.key == tier.key())
    }

    pub fn is_ready(&self, tier: Tier) -> bool {
        self.tier(tier)
            .is_some_and(|status| status.state == TierState::Ready)
    }
}

// ---------------------------------------------------------------------------
// The index directory
// ---------------------------------------------------------------------------

/// Handle on an index directory. Cheap to create; does no I/O until asked.
#[derive(Debug, Clone)]
pub struct GraphIndex {
    root: PathBuf,
}

impl GraphIndex {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn meta_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn vertices_path(&self) -> PathBuf {
        self.root.join("vertices.idx")
    }

    pub fn edges_path(&self) -> PathBuf {
        self.root.join("edges.idx")
    }

    pub fn ranks_path(&self) -> PathBuf {
        self.root.join("ranks.bin")
    }

    pub fn inbound_offsets_path(&self) -> PathBuf {
        self.root.join("inbound.off")
    }

    pub fn inbound_sources_path(&self) -> PathBuf {
        self.root.join("inbound.src")
    }

    pub fn load_meta(&self) -> IndexMeta {
        let Ok(raw) = std::fs::read_to_string(self.meta_path()) else {
            return IndexMeta::default();
        };
        match serde_json::from_str::<IndexMeta>(&raw) {
            Ok(meta) if meta.format == FORMAT_VERSION => meta,
            Ok(meta) => {
                log::warn!(
                    "index format {} is not {FORMAT_VERSION} — treating the index as absent",
                    meta.format
                );
                IndexMeta::default()
            }
            Err(error) => {
                log::warn!("could not read {}: {error}", self.meta_path().display());
                IndexMeta::default()
            }
        }
    }

    fn save_meta(&self, meta: &IndexMeta) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let raw = serde_json::to_string_pretty(meta)?;
        std::fs::write(self.meta_path(), raw)
            .with_context(|| format!("не вдалося зберегти {}", self.meta_path().display()))?;
        Ok(())
    }

    fn tier_bytes(&self, tier: Tier) -> u64 {
        tier.files(&self.root)
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|m| m.len())
            .sum()
    }

    fn tier_state(&self, tier: Tier, meta: &IndexMeta, sources: &GraphSources) -> TierState {
        let Some(record) = meta.tiers.get(tier.key()) else {
            return TierState::Missing;
        };
        if tier.files(&self.root).iter().any(|path| !path.is_file()) {
            return TierState::Missing;
        }
        if record.sources != tier.sources(sources) {
            return TierState::Stale;
        }
        TierState::Ready
    }

    /// Everything the UI needs to render the data panel.
    pub fn status(&self, sources: &GraphSources) -> IndexStatus {
        let meta = self.load_meta();
        let source_status = sources.status();
        let crawl = sources.crawl();
        let mut blockers = Vec::new();
        for status in &source_status {
            let Some(file) = GraphFile::parse(&status.kind) else {
                continue;
            };
            if !status.exists {
                blockers.push(format!(
                    "Немає файла {} — завантажте {} (≈{}) і покладіть сюди: {}",
                    status.kind,
                    file.archive_name(&crawl),
                    human_bytes(file.download_bytes()),
                    sources.expected_dir().display(),
                ));
            } else if status.compressed && status.must_be_unpacked {
                blockers.push(format!(
                    "Файл {} лишився стиснутим (.gz). Запити читають його за зміщенням, а gzip так не вміє — розпакуйте його (≈{} після розпакування).",
                    status.kind,
                    human_bytes(file.unpacked_bytes()),
                ));
            }
        }

        let tiers = Tier::ALL
            .into_iter()
            .map(|tier| TierStatus {
                key: tier.key().to_string(),
                label: tier.label().to_string(),
                description: tier.description().to_string(),
                state: self.tier_state(tier, &meta, sources),
                bytes: self.tier_bytes(tier),
                estimated_bytes: tier.estimated_bytes(sources),
                estimated_temp_bytes: tier.estimated_temp_bytes(sources),
                built_at: meta
                    .tiers
                    .get(tier.key())
                    .map(|record| record.built_at.clone()),
            })
            .collect();

        IndexStatus {
            root: self.root.to_string_lossy().into_owned(),
            node_count: meta.node_count,
            edge_count: meta.edge_count,
            total_bytes: Tier::ALL.into_iter().map(|t| self.tier_bytes(t)).sum(),
            free_bytes: available_space(&self.root),
            tiers,
            sources: source_status,
            blockers,
            setup: setup_guide(&crawl),
        }
    }

    /// Delete one tier's files and forget it in the metadata.
    pub fn drop_tier(&self, tier: Tier) -> Result<()> {
        for path in tier.files(&self.root) {
            if path.is_file() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("не вдалося видалити {}", path.display()))?;
            }
        }
        let mut meta = self.load_meta();
        meta.tiers.remove(tier.key());
        self.save_meta(&meta)
    }

    /// Build the requested tiers, in dependency order.
    ///
    /// [`Tier::Ranks`] and [`Tier::Inbound`] both need the vertex index, so
    /// [`Tier::Lookup`] is added automatically when it is missing or stale.
    pub fn build(
        &self,
        sources: &GraphSources,
        wanted: &[Tier],
        progress: &Progress,
    ) -> Result<IndexMeta> {
        let mut meta = self.load_meta();
        let mut plan: Vec<Tier> = Vec::new();

        let lookup_ready = self.tier_state(Tier::Lookup, &meta, sources) == TierState::Ready;
        if wanted.contains(&Tier::Lookup) || !lookup_ready {
            plan.push(Tier::Lookup);
        }
        for tier in [Tier::Ranks, Tier::Inbound] {
            if wanted.contains(&tier) {
                plan.push(tier);
            }
        }
        if plan.is_empty() {
            return Ok(meta);
        }

        let crawl = sources.crawl();
        for (kind, path) in sources.list() {
            let file = GraphFile::parse(kind);
            if !path.is_file() {
                bail!(
                    "не знайдено файл {kind}:\n  {}\n\n{}",
                    path.display(),
                    crate::data_source::instructions(&crawl, &sources.expected_dir()),
                );
            }
            // `ranks` is streamed once and never seeked, so gzip is fine there.
            if is_compressed(path) && file.is_some_and(GraphFile::must_be_unpacked) {
                bail!(
                    "файл {kind} стиснутий:\n  {}\n\n\
                     Запити читають цей файл за зміщенням, а .gz такого не дає — розпакуйте його\n\
                     (стане ≈{}) і повторіть. Файл ranks розпаковувати не треба.",
                    path.display(),
                    human_bytes(file.map(GraphFile::unpacked_bytes).unwrap_or(0)),
                );
            }
        }
        self.check_free_space(sources, &plan)?;
        std::fs::create_dir_all(&self.root)?;

        let total_work: u64 = plan.iter().map(|tier| tier.work_units(sources)).sum();
        let mut done_work: u64 = 0;

        for tier in plan {
            let base = done_work as f64 / total_work as f64;
            let weight = tier.work_units(sources) as f64 / total_work as f64;
            self.build_tier(tier, sources, &mut meta, progress, base, weight)?;
            done_work += tier.work_units(sources);
            self.save_meta(&meta)?;
        }
        let _ = std::fs::remove_dir(self.tmp_dir());
        Ok(meta)
    }

    fn build_tier(
        &self,
        tier: Tier,
        sources: &GraphSources,
        meta: &mut IndexMeta,
        progress: &Progress,
        base: f64,
        weight: f64,
    ) -> Result<()> {
        match tier {
            Tier::Lookup => {
                let vertices_summary = vertices::build(
                    &sources.vertices,
                    &self.vertices_path(),
                    progress,
                    base,
                    weight * 0.95,
                )?;
                if !vertices_summary.sorted_by_name {
                    log::warn!(
                        "vertices file is not sorted by domain — name lookups will fall back to a scan"
                    );
                }
                let edges_summary = edges::build(
                    &sources.edges,
                    &self.edges_path(),
                    progress,
                    base + weight * 0.95,
                    weight * 0.05,
                )?;
                meta.node_count = vertices_summary.node_count;
                meta.node_capacity = u64::from(vertices_summary.max_id) + 1;
                if meta.edge_count == 0 {
                    meta.edge_count = edges_summary.estimated_edges;
                }
            }
            Tier::Ranks => {
                let vertex_index = VertexIndex::open(&self.vertices_path(), &sources.vertices)?;
                let capacity = if meta.node_capacity > 0 {
                    meta.node_capacity
                } else {
                    vertex_index.node_count
                };
                ranks::build(
                    &sources.ranks,
                    &self.ranks_path(),
                    &vertex_index,
                    capacity,
                    &self.tmp_dir(),
                    progress,
                    base,
                    weight,
                )?;
            }
            Tier::Inbound => {
                let capacity = if meta.node_capacity > 0 {
                    meta.node_capacity
                } else {
                    VertexIndex::open(&self.vertices_path(), &sources.vertices)?.node_count
                };
                let estimated = if meta.edge_count > 0 {
                    meta.edge_count
                } else {
                    sources.estimated_edges()
                };
                let result = inbound::build(
                    &sources.edges,
                    &self.inbound_offsets_path(),
                    &self.inbound_sources_path(),
                    capacity,
                    estimated,
                    &self.tmp_dir(),
                    progress,
                    base,
                    weight,
                )?;
                meta.edge_count = result.edge_count;
            }
        }

        meta.format = FORMAT_VERSION;
        meta.tiers.insert(
            tier.key().to_string(),
            TierRecord {
                built_at: now_rfc3339(),
                built_by: crate::meta::VERSION.to_string(),
                bytes: self.tier_bytes(tier),
                sources: tier.sources(sources),
            },
        );
        Ok(())
    }

    fn check_free_space(&self, sources: &GraphSources, plan: &[Tier]) -> Result<()> {
        let needed: u64 = plan
            .iter()
            .map(|tier| tier.estimated_bytes(sources))
            .sum::<u64>()
            + plan
                .iter()
                .map(|tier| tier.estimated_temp_bytes(sources))
                .max()
                .unwrap_or(0);
        let free = available_space(&self.root);
        if free == 0 {
            // Unknown rather than full: warn, don't block.
            log::warn!("could not determine free space for {}", self.root.display());
            return Ok(());
        }
        if free < needed {
            bail!(
                "недостатньо місця для індексу в\n  {}\n\nПотрібно ≈{}, вільно {}.\n\
                 Оберіть інший диск для індексу або зніміть найважчий рівень «{}».",
                self.root.display(),
                human_bytes(needed),
                human_bytes(free),
                Tier::Inbound.label(),
            );
        }
        Ok(())
    }
}

/// Free space on the volume holding `path`, 0 when it cannot be determined.
pub fn available_space(path: &Path) -> u64 {
    let mut probe = path.to_path_buf();
    loop {
        if probe.exists() {
            return fs2::available_space(&probe).unwrap_or(0);
        }
        if !probe.pop() {
            return 0;
        }
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    if bytes == 0 {
        return "0 Б".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit >= 3 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_graph(dir: &Path) -> GraphSources {
        let vertices = dir.join("vertices.txt");
        let edges = dir.join("edges.txt");
        let ranks = dir.join("ranks.txt");
        let mut vertex_text = String::new();
        for i in 0..600u32 {
            vertex_text.push_str(&format!("{i}\tcom.site{i:03}\t1\n"));
        }
        std::fs::write(&vertices, vertex_text).expect("vertices");

        let mut edge_text = String::new();
        for from in 0..600u32 {
            for step in 0..3u32 {
                edge_text.push_str(&format!("{from}\t{}\n", (from + 1 + step * 7) % 600));
            }
        }
        std::fs::write(&edges, edge_text).expect("edges");

        let mut rank_text =
            String::from("#harmonicc_pos\t#harmonicc_val\t#pr_pos\t#pr_val\t#host_rev\t#n_hosts\n");
        for i in 0..600u32 {
            rank_text.push_str(&format!(
                "{}\t1.0\t{}\t1.0E-9\tcom.site{i:03}\t1\n",
                i + 1,
                i + 1
            ));
        }
        std::fs::write(&ranks, rank_text).expect("ranks");

        GraphSources {
            vertices,
            edges,
            ranks,
        }
    }

    #[test]
    fn builds_every_tier_and_reports_them_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = tiny_graph(dir.path());
        let index = GraphIndex::new(dir.path().join("index"));

        assert!(
            Tier::ALL
                .into_iter()
                .all(|tier| index.status(&sources).tier(tier).unwrap().state == TierState::Missing),
            "a fresh directory must report every tier missing"
        );

        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build all tiers");

        let status = index.status(&sources);
        assert_eq!(status.node_count, 600);
        assert!(status.edge_count > 0);
        assert!(
            status.blockers.is_empty(),
            "unexpected blockers: {:?}",
            status.blockers
        );
        for tier in Tier::ALL {
            let tier_status = status.tier(tier).expect("tier status");
            assert_eq!(
                tier_status.state,
                TierState::Ready,
                "{} not ready",
                tier.key()
            );
            assert!(tier_status.bytes > 0, "{} wrote nothing", tier.key());
        }
    }

    #[test]
    fn a_changed_source_file_marks_its_tiers_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = tiny_graph(dir.path());
        let index = GraphIndex::new(dir.path().join("index"));
        index
            .build(&sources, &[Tier::Lookup, Tier::Ranks], &Progress::silent())
            .expect("build");
        assert!(index.status(&sources).is_ready(Tier::Ranks));

        let mut ranks = std::fs::read_to_string(&sources.ranks).expect("read");
        ranks.push_str("601\t1.0\t601\t1.0E-9\tcom.site600\t1\n");
        std::fs::write(&sources.ranks, ranks).expect("write");

        let status = index.status(&sources);
        assert_eq!(status.tier(Tier::Ranks).unwrap().state, TierState::Stale);
        // The lookup tier does not read the ranks file, so it stays valid.
        assert_eq!(status.tier(Tier::Lookup).unwrap().state, TierState::Ready);
    }

    #[test]
    fn dropping_a_tier_removes_its_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = tiny_graph(dir.path());
        let index = GraphIndex::new(dir.path().join("index"));
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("build");
        assert!(index.inbound_sources_path().is_file());

        index.drop_tier(Tier::Inbound).expect("drop");
        assert!(!index.inbound_sources_path().is_file());
        assert_eq!(
            index.status(&sources).tier(Tier::Inbound).unwrap().state,
            TierState::Missing
        );
        assert!(
            index.status(&sources).is_ready(Tier::Lookup),
            "unrelated tiers must survive"
        );
    }

    #[test]
    fn refuses_a_compressed_vertices_file_with_an_actionable_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sources = tiny_graph(dir.path());
        let gz = dir.path().join("vertices.txt.gz");
        std::fs::write(&gz, b"not really gzip").expect("write");
        sources.vertices = gz;

        let index = GraphIndex::new(dir.path().join("index"));
        let error = index
            .build(&sources, &[Tier::Lookup], &Progress::silent())
            .expect_err("a seeked source cannot stay gzipped");
        let text = format!("{error}");
        assert!(text.contains("розпакуйте"), "must say what to do: {text}");
        assert!(
            text.contains("ranks розпаковувати не треба"),
            "must say which file is exempt: {text}"
        );

        let status = index.status(&sources);
        assert!(
            status.blockers.iter().any(|b| b.contains(".gz")),
            "{:?}",
            status.blockers
        );
    }

    #[test]
    fn a_gzipped_ranks_file_is_accepted_because_it_is_only_ever_streamed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = tiny_graph(dir.path());

        // Same fixture, but with ranks left compressed as downloaded.
        let plain = std::fs::read(&sources.ranks).expect("read ranks");
        let gz_path = dir.path().join("ranks.txt.gz");
        {
            use std::io::Write;
            let file = std::fs::File::create(&gz_path).expect("create gz");
            let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            encoder.write_all(&plain).expect("compress");
            encoder.finish().expect("finish");
        }
        let sources = GraphSources {
            ranks: gz_path,
            ..sources
        };

        let index = GraphIndex::new(dir.path().join("index"));
        index
            .build(&sources, &Tier::ALL, &Progress::silent())
            .expect("a gzipped ranks file must not block indexing");
        assert!(index.status(&sources).is_ready(Tier::Ranks));

        // And it is not reported as something the user must fix.
        let status = index.status(&sources);
        assert!(
            status.blockers.is_empty(),
            "gzipped ranks must not be a blocker: {:?}",
            status.blockers
        );

        // The answers must be the same as from the plain-text build.
        let engine = crate::query::Engine::open(&index, &sources).expect("open");
        let report = engine
            .query("site003.com", crate::query::QueryOptions::default())
            .expect("query");
        assert!(report.found);
        let rank = report.rank.expect("rank came from the gzipped file");
        // Ranks are stored as f32, so compare within that precision.
        assert!(
            (rank - 1.0e-9).abs() < 1e-15,
            "unexpected rank {rank:e} from the gzipped ranks file"
        );
    }

    #[test]
    fn human_bytes_reads_like_a_person_wrote_it() {
        assert_eq!(human_bytes(0), "0 Б");
        assert_eq!(human_bytes(1023), "1023 Б");
        assert_eq!(human_bytes(1024), "1 КБ");
        assert_eq!(human_bytes(16 * 1024 * 1024 * 1024), "16.0 ГБ");
    }
}
