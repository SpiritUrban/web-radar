//! Streaming processor for Common Crawl domain-level web graphs.
//!
//! Pipeline (memory-efficient, suitable for multi‑GB edge files):
//! 1. Scan **vertices** → map target reverse-domains to node IDs.
//! 2. Stream **edges** → collect inbound *and* outbound neighbors per target.
//! 3. Re-scan **vertices** → resolve neighbor IDs to reverse domains.
//! 4. Scan **ranks** → attach pagerank / harmonic to targets and neighbors.
//! 5. Write `results/{reversed-domain}.json` per target (own rank + links).

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};
use flate2::read::MultiGzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, info, warn};
use memchr::memchr;
use serde::Serialize;

use crate::config::{Config, RankMetric, ResolvedTarget};
use crate::reverse::{from_reverse, reverse_to_filename};

/// One linked domain with its rank (inbound source or outbound destination).
#[derive(Debug, Clone, Serialize)]
pub struct LinkEntry {
    /// Domain in normal form (`other.com`).
    pub domain: String,
    /// Selected rank metric of that domain.
    pub rank: f64,
}

/// Full report for one configured target domain.
#[derive(Debug, Clone, Serialize)]
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
}

impl TargetResult {
    fn not_found(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            found: false,
            rank: None,
            inbound: Vec::new(),
            outbound: Vec::new(),
        }
    }
}

/// Inbound + outbound adjacency collected in one edges pass.
struct EdgeSets {
    /// target_id → set of source_ids (who links to target)
    inbound: AHashMap<u32, AHashSet<u32>>,
    /// target_id → set of dest_ids (where target links to)
    outbound: AHashMap<u32, AHashSet<u32>>,
}

/// Buffered line reader over plain or gzip-compressed text.
struct LineReader {
    inner: Box<dyn BufRead>,
    path: PathBuf,
    /// Total compressed/raw file size for progress (0 if unknown).
    file_size: u64,
}

impl LineReader {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let is_gz = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"));

        let reader: Box<dyn BufRead> = if is_gz {
            // MultiGzDecoder handles concatenated gzip members (common on S3 dumps).
            let decoder = MultiGzDecoder::new(file);
            Box::new(BufReader::with_capacity(1024 * 1024, decoder))
        } else {
            Box::new(BufReader::with_capacity(1024 * 1024, file))
        };

        Ok(Self {
            inner: reader,
            path: path.to_path_buf(),
            file_size,
        })
    }

    fn path_display(&self) -> String {
        self.path.display().to_string()
    }
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) {msg}",
    )
    .expect("progress style")
    .progress_chars("=>-")
}

fn make_bar(total: u64, msg: &str) -> ProgressBar {
    let bar = if total > 0 {
        ProgressBar::new(total)
    } else {
        ProgressBar::new_spinner()
    };
    bar.set_style(progress_style());
    bar.set_message(msg.to_string());
    bar.enable_steady_tick(std::time::Duration::from_millis(200));
    bar
}

/// Parse a decimal `u32` from ASCII bytes without intermediate String.
#[inline]
fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    Some(n)
}

/// Parse an f64 from ASCII bytes.
#[inline]
fn parse_f64(bytes: &[u8]) -> Option<f64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Split a tab-separated line into fields (zero-copy).
#[inline]
fn split_tabs(line: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::with_capacity(6);
    let mut rest = line;
    while let Some(pos) = memchr(b'\t', rest) {
        fields.push(&rest[..pos]);
        rest = &rest[pos + 1..];
    }
    fields.push(rest);
    fields
}

/// Strip trailing CR (Windows line endings).
#[inline]
fn trim_cr(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

// ---------------------------------------------------------------------------
// Pass 1 — resolve target node IDs from vertices
// ---------------------------------------------------------------------------

/// Vertices line: `id \t reverse_domain \t n_hosts`
fn pass_resolve_targets(
    vertices_path: &Path,
    targets: &[ResolvedTarget],
    progress: &dyn Fn(u64, u64),
) -> Result<AHashMap<u32, String>> {
    // reverse_domain → original domain (from config)
    let wanted: AHashMap<&str, &str> = targets
        .iter()
        .map(|t| (t.reverse.as_str(), t.domain.as_str()))
        .collect();

    let mut reader = LineReader::open(vertices_path)?;
    let bar = make_bar(reader.file_size, "vertices: resolve targets");
    info!(
        "Pass 1/4: scanning vertices for {} target(s) — {}",
        wanted.len(),
        reader.path_display()
    );

    // node_id → reverse_domain
    let mut id_to_rev: AHashMap<u32, String> = AHashMap::with_capacity(wanted.len());
    let mut line_buf = Vec::with_capacity(256);
    let mut lines: u64 = 0;
    let mut bytes_approx: u64 = 0;

    loop {
        line_buf.clear();
        let n = reader
            .inner
            .read_until(b'\n', &mut line_buf)
            .with_context(|| format!("read error in {}", reader.path_display()))?;
        if n == 0 {
            break;
        }
        bytes_approx += n as u64;
        if lines & 0xFFFF == 0 {
            bar.set_position(bytes_approx.min(reader.file_size.max(1)));
            progress(bytes_approx, reader.file_size);
        }
        lines += 1;

        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
        }
        trim_cr(&mut line_buf);
        if line_buf.is_empty() || line_buf[0] == b'#' {
            continue;
        }

        let fields = split_tabs(&line_buf);
        if fields.len() < 2 {
            continue;
        }
        let Some(id) = parse_u32(fields[0]) else {
            continue;
        };
        let rev = match std::str::from_utf8(fields[1]) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if wanted.contains_key(rev) {
            debug!("target '{rev}' → node id {id}");
            id_to_rev.insert(id, rev.to_owned());
            if id_to_rev.len() == wanted.len() {
                // All targets found — early exit is safe for a single vertices file.
                break;
            }
        }
    }

    progress(reader.file_size, reader.file_size);
    bar.finish_with_message(format!(
        "vertices: found {}/{} targets ({lines} lines scanned)",
        id_to_rev.len(),
        wanted.len()
    ));

    for t in targets {
        if !id_to_rev.values().any(|r| r == &t.reverse) {
            warn!(
                "target domain '{}' (rev '{}') not found in vertices file \
                 — will write a stub result with found=false",
                t.domain, t.reverse
            );
        }
    }

    // Empty is OK: run() still writes not-found stubs for every configured target.
    info!(
        "resolved {}/{} target node id(s)",
        id_to_rev.len(),
        targets.len()
    );
    Ok(id_to_rev)
}

// ---------------------------------------------------------------------------
// Pass 2 — stream edges, collect inbound sources + outbound destinations
// ---------------------------------------------------------------------------

/// Edges line: `from_id \t to_id`
///
/// Single pass over the (often multi‑GB) edges file:
/// - if `to_id` is a target → record inbound source `from_id`
/// - if `from_id` is a target → record outbound destination `to_id`
fn pass_collect_edges(
    edges_path: &Path,
    target_ids: &AHashMap<u32, String>,
    progress: &dyn Fn(u64, u64),
) -> Result<EdgeSets> {
    let mut inbound: AHashMap<u32, AHashSet<u32>> = target_ids
        .keys()
        .map(|&id| (id, AHashSet::new()))
        .collect();
    let mut outbound: AHashMap<u32, AHashSet<u32>> = target_ids
        .keys()
        .map(|&id| (id, AHashSet::new()))
        .collect();

    let mut reader = LineReader::open(edges_path)?;
    let bar = make_bar(reader.file_size, "edges: collect inbound + outbound");
    info!(
        "Pass 2/4: streaming edges (inbound + outbound) — {}",
        reader.path_display()
    );

    let mut line_buf = Vec::with_capacity(64);
    let mut lines: u64 = 0;
    let mut inbound_hits: u64 = 0;
    let mut outbound_hits: u64 = 0;
    let mut bytes_approx: u64 = 0;

    loop {
        line_buf.clear();
        let n = reader
            .inner
            .read_until(b'\n', &mut line_buf)
            .with_context(|| format!("read error in {}", reader.path_display()))?;
        if n == 0 {
            break;
        }
        bytes_approx += n as u64;
        // Update progress every ~65k lines to keep overhead low.
        if lines & 0xFFFF == 0 {
            bar.set_position(bytes_approx.min(reader.file_size.max(1)));
            progress(bytes_approx, reader.file_size);
        }
        lines += 1;

        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
        }
        trim_cr(&mut line_buf);
        if line_buf.is_empty() || line_buf[0] == b'#' {
            continue;
        }

        // Fast two-field parse: find first tab, parse both sides as u32.
        let Some(tab) = memchr(b'\t', &line_buf) else {
            continue;
        };
        let Some(from_id) = parse_u32(&line_buf[..tab]) else {
            continue;
        };
        let to_bytes = &line_buf[tab + 1..];
        // Tolerate extra trailing fields / whitespace.
        let to_end = to_bytes
            .iter()
            .position(|&b| b == b'\t' || b == b' ')
            .unwrap_or(to_bytes.len());
        let Some(to_id) = parse_u32(&to_bytes[..to_end]) else {
            continue;
        };

        // Skip self-loops for both directions.
        if from_id == to_id {
            continue;
        }

        if let Some(sources) = inbound.get_mut(&to_id) {
            sources.insert(from_id);
            inbound_hits += 1;
        }
        if let Some(dests) = outbound.get_mut(&from_id) {
            dests.insert(to_id);
            outbound_hits += 1;
        }
    }

    progress(reader.file_size, reader.file_size);
    bar.finish_with_message(format!(
        "edges: {inbound_hits} inbound + {outbound_hits} outbound hits over {lines} edges"
    ));

    for (&tid, rev) in target_ids {
        let in_n = inbound.get(&tid).map(|s| s.len()).unwrap_or(0);
        let out_n = outbound.get(&tid).map(|s| s.len()).unwrap_or(0);
        info!("  target id {tid} ({rev}): {in_n} inbound, {out_n} outbound");
    }

    Ok(EdgeSets { inbound, outbound })
}

// ---------------------------------------------------------------------------
// Pass 3 — resolve neighbor IDs → reverse domain names
// ---------------------------------------------------------------------------

fn pass_resolve_names(
    vertices_path: &Path,
    needed_ids: &AHashSet<u32>,
    progress: &dyn Fn(u64, u64),
) -> Result<AHashMap<u32, String>> {
    if needed_ids.is_empty() {
        return Ok(AHashMap::new());
    }

    let mut reader = LineReader::open(vertices_path)?;
    let bar = make_bar(reader.file_size, "vertices: resolve neighbor names");
    info!(
        "Pass 3/4: resolving {} neighbor name(s) — {}",
        needed_ids.len(),
        reader.path_display()
    );

    let mut id_to_rev: AHashMap<u32, String> =
        AHashMap::with_capacity(needed_ids.len());
    let mut line_buf = Vec::with_capacity(256);
    let mut lines: u64 = 0;
    let mut bytes_approx: u64 = 0;

    loop {
        line_buf.clear();
        let n = reader
            .inner
            .read_until(b'\n', &mut line_buf)
            .with_context(|| format!("read error in {}", reader.path_display()))?;
        if n == 0 {
            break;
        }
        bytes_approx += n as u64;
        if lines & 0xFFFF == 0 {
            bar.set_position(bytes_approx.min(reader.file_size.max(1)));
            progress(bytes_approx, reader.file_size);
        }
        lines += 1;

        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
        }
        trim_cr(&mut line_buf);
        if line_buf.is_empty() || line_buf[0] == b'#' {
            continue;
        }

        let fields = split_tabs(&line_buf);
        if fields.len() < 2 {
            continue;
        }
        let Some(id) = parse_u32(fields[0]) else {
            continue;
        };
        if !needed_ids.contains(&id) {
            continue;
        }
        let rev = match std::str::from_utf8(fields[1]) {
            Ok(s) => s.to_owned(),
            Err(_) => continue,
        };
        id_to_rev.insert(id, rev);

        if id_to_rev.len() == needed_ids.len() {
            break;
        }
    }

    progress(reader.file_size, reader.file_size);
    bar.finish_with_message(format!(
        "vertices: resolved {}/{} names",
        id_to_rev.len(),
        needed_ids.len()
    ));

    if id_to_rev.len() < needed_ids.len() {
        warn!(
            "could not resolve {} source id(s) to domain names",
            needed_ids.len() - id_to_rev.len()
        );
    }

    Ok(id_to_rev)
}

// ---------------------------------------------------------------------------
// Pass 4 — load ranks for reverse domains of interest
// ---------------------------------------------------------------------------

/// Ranks line (after optional header):
/// `#harmonicc_pos  #harmonicc_val  #pr_pos  #pr_val  #host_rev  [#n_hosts]`
fn pass_load_ranks(
    ranks_path: &Path,
    needed_revs: &AHashSet<&str>,
    metric: RankMetric,
    progress: &dyn Fn(u64, u64),
) -> Result<AHashMap<String, f64>> {
    if needed_revs.is_empty() {
        return Ok(AHashMap::new());
    }

    let mut reader = LineReader::open(ranks_path)?;
    let bar = make_bar(reader.file_size, "ranks: load target + neighbor ranks");
    info!(
        "Pass 4/4: loading {} ranks ({}) — {}",
        needed_revs.len(),
        metric.as_str(),
        reader.path_display()
    );

    // Column indices (0-based) after detecting header, with safe defaults
    // matching the Common Crawl join_ranks output format.
    let mut col_harmonic_val: usize = 1;
    let mut col_pr_val: usize = 3;
    let mut col_host_rev: usize = 4;
    let mut header_seen = false;

    let mut ranks: AHashMap<String, f64> =
        AHashMap::with_capacity(needed_revs.len());
    let mut line_buf = Vec::with_capacity(256);
    let mut lines: u64 = 0;
    let mut bytes_approx: u64 = 0;

    loop {
        line_buf.clear();
        let n = reader
            .inner
            .read_until(b'\n', &mut line_buf)
            .with_context(|| format!("read error in {}", reader.path_display()))?;
        if n == 0 {
            break;
        }
        bytes_approx += n as u64;
        if lines & 0xFFFF == 0 {
            bar.set_position(bytes_approx.min(reader.file_size.max(1)));
            progress(bytes_approx, reader.file_size);
        }
        lines += 1;

        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
        }
        trim_cr(&mut line_buf);
        if line_buf.is_empty() {
            continue;
        }

        // Header line (starts with '#').
        if line_buf[0] == b'#' {
            if !header_seen {
                header_seen = true;
                if let Ok(header) = std::str::from_utf8(&line_buf) {
                    parse_ranks_header(
                        header,
                        &mut col_harmonic_val,
                        &mut col_pr_val,
                        &mut col_host_rev,
                    );
                    debug!(
                        "ranks header columns: harmonic_val={col_harmonic_val}, \
                         pr_val={col_pr_val}, host_rev={col_host_rev}"
                    );
                }
            }
            continue;
        }

        let fields = split_tabs(&line_buf);
        let max_col = col_host_rev
            .max(col_pr_val)
            .max(col_harmonic_val);
        if fields.len() <= max_col {
            continue;
        }

        let rev = match std::str::from_utf8(fields[col_host_rev]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !needed_revs.contains(rev) {
            continue;
        }

        let rank_col = match metric {
            RankMetric::Pagerank => col_pr_val,
            RankMetric::Harmonic => col_harmonic_val,
        };
        let Some(rank) = parse_f64(fields[rank_col]) else {
            continue;
        };
        ranks.insert(rev.to_owned(), rank);

        if ranks.len() == needed_revs.len() {
            break;
        }
    }

    progress(reader.file_size, reader.file_size);
    bar.finish_with_message(format!(
        "ranks: loaded {}/{} ({})",
        ranks.len(),
        needed_revs.len(),
        metric.as_str()
    ));

    if ranks.len() < needed_revs.len() {
        warn!(
            "missing ranks for {} domain(s) (will use 0.0)",
            needed_revs.len() - ranks.len()
        );
    }

    Ok(ranks)
}

fn parse_ranks_header(
    header: &str,
    col_harmonic_val: &mut usize,
    col_pr_val: &mut usize,
    col_host_rev: &mut usize,
) {
    for (i, col) in header.trim_start_matches('#').split('\t').enumerate() {
        let name = col.trim().trim_start_matches('#').to_ascii_lowercase();
        match name.as_str() {
            "harmonicc_val" | "harmonic_val" | "harmonic" => *col_harmonic_val = i,
            "pr_val" | "pagerank" | "pr" => *col_pr_val = i,
            "host_rev" | "domain_rev" | "rev_host" | "rev_domain" => *col_host_rev = i,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Write results
// ---------------------------------------------------------------------------

fn sort_links(entries: &mut [LinkEntry]) {
    entries.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.domain.cmp(&b.domain))
    });
}

fn links_from_ids(
    ids: Option<&AHashSet<u32>>,
    id_to_rev: &AHashMap<u32, String>,
    ranks: &AHashMap<String, f64>,
) -> Vec<LinkEntry> {
    let mut entries: Vec<LinkEntry> = ids
        .map(|set| {
            set.iter()
                .filter_map(|&nid| {
                    let rev = id_to_rev.get(&nid)?;
                    let rank = ranks.get(rev).copied().unwrap_or(0.0);
                    Some(LinkEntry {
                        domain: from_reverse(rev),
                        rank,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    sort_links(&mut entries);
    entries
}

fn write_json_result(results_dir: &Path, rev_target: &str, result: &TargetResult) -> Result<PathBuf> {
    let filename = reverse_to_filename(rev_target) + ".json";
    let out_path = results_dir.join(&filename);

    let file = File::create(&out_path)
        .with_context(|| format!("failed to create {}", out_path.display()))?;
    let mut writer = io::BufWriter::with_capacity(256 * 1024, file);
    serde_json::to_writer_pretty(&mut writer, result)
        .with_context(|| format!("failed to serialize {}", out_path.display()))?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    Ok(pretty_path(&out_path))
}

/// Write `found: false` stubs for targets missing from the vertices graph.
fn write_not_found_stubs(
    results_dir: &Path,
    targets: &[ResolvedTarget],
    found_revs: &AHashSet<&str>,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(results_dir)
        .with_context(|| format!("failed to create {}", results_dir.display()))?;

    let mut written = Vec::new();
    let mut missing: Vec<&ResolvedTarget> = targets
        .iter()
        .filter(|t| !found_revs.contains(t.reverse.as_str()))
        .collect();
    missing.sort_by(|a, b| a.reverse.cmp(&b.reverse));

    for t in missing {
        let result = TargetResult::not_found(&t.domain);
        let abs = write_json_result(results_dir, &t.reverse, &result)?;
        info!(
            "wrote not-found stub for '{}' → {}",
            t.domain,
            abs.display()
        );
        written.push(abs);
    }
    Ok(written)
}

fn write_found_results(
    results_dir: &Path,
    target_ids: &AHashMap<u32, String>,
    edges: &EdgeSets,
    id_to_rev: &AHashMap<u32, String>,
    ranks: &AHashMap<String, f64>,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(results_dir)
        .with_context(|| format!("failed to create {}", results_dir.display()))?;

    let mut written = Vec::new();

    // Stable order by reverse domain so multi-target runs are predictable.
    let mut ordered: Vec<(&u32, &String)> = target_ids.iter().collect();
    ordered.sort_by(|a, b| a.1.cmp(b.1));

    for (&target_id, rev_target) in ordered {
        let domain = from_reverse(rev_target);
        let rank = ranks.get(rev_target.as_str()).copied().unwrap_or(0.0);

        let inbound = links_from_ids(edges.inbound.get(&target_id), id_to_rev, ranks);
        let outbound = links_from_ids(edges.outbound.get(&target_id), id_to_rev, ranks);

        let result = TargetResult {
            domain,
            found: true,
            rank: Some(rank),
            inbound,
            outbound,
        };

        let abs = write_json_result(results_dir, rev_target, &result)?;
        info!(
            "wrote found=true, rank={:.6e}, {} inbound, {} outbound → {}",
            rank,
            result.inbound.len(),
            result.outbound.len(),
            abs.display()
        );
        written.push(abs);
    }

    Ok(written)
}

/// Absolute path without Windows `\\?\` prefix (nicer logs).
fn pretty_path(p: &Path) -> PathBuf {
    let abs = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        abs
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full multi-pass pipeline using the given config.
pub fn run(cfg: &Config) -> Result<()> {
    run_with_progress(cfg, |_, _, _| {})
}

/// Run the pipeline and report byte progress for each streaming phase.
pub fn run_with_progress<F>(cfg: &Config, progress: F) -> Result<()>
where
    F: Fn(&str, u64, u64) + Sync,
{
    let targets = cfg.resolved_targets()?;
    info!(
        "processing {} target(s) with rank metric '{}'",
        targets.len(),
        cfg.rank_metric.as_str()
    );
    for t in &targets {
        info!("  • {}  (rev: {})", t.domain, t.reverse);
    }

    // Pass 1
    let target_ids = pass_resolve_targets(&cfg.paths.vertices, &targets, &|done, total| progress("vertices_targets", done, total))?;
    let found_revs: AHashSet<&str> = target_ids.values().map(|s| s.as_str()).collect();

    // Always materialize not-found stubs so missing domains are visible in results/.
    let mut written = write_not_found_stubs(&cfg.paths.results_dir, &targets, &found_revs)?;

    if target_ids.is_empty() {
        info!(
            "none of the configured targets were found in the vertices graph \
             — only not-found stubs written"
        );
        print_done_summary(&cfg.paths.results_dir, &written);
        return Ok(());
    }

    // Pass 2 — inbound + outbound in one edges scan
    let edges = pass_collect_edges(&cfg.paths.edges, &target_ids, &|done, total| progress("edges", done, total))?;

    // Collect every neighbor id we must resolve (sources + destinations).
    let mut needed_ids: AHashSet<u32> = AHashSet::new();
    for set in edges.inbound.values() {
        needed_ids.extend(set.iter().copied());
    }
    for set in edges.outbound.values() {
        needed_ids.extend(set.iter().copied());
    }
    info!("unique neighbor domains to resolve: {}", needed_ids.len());

    // Pass 3
    let id_to_rev = pass_resolve_names(&cfg.paths.vertices, &needed_ids, &|done, total| progress("vertices_neighbors", done, total))?;

    // Pass 4 — ranks for neighbors + the targets themselves
    let mut needed_revs: AHashSet<&str> = id_to_rev.values().map(|s| s.as_str()).collect();
    for rev in target_ids.values() {
        needed_revs.insert(rev.as_str());
    }
    let ranks = pass_load_ranks(&cfg.paths.ranks, &needed_revs, cfg.rank_metric, &|done, total| progress("ranks", done, total))?;

    progress("writing", 0, 1);

    // Write JSON for found targets
    let found_paths = write_found_results(
        &cfg.paths.results_dir,
        &target_ids,
        &edges,
        &id_to_rev,
        &ranks,
    )?;
    written.extend(found_paths);
    written.sort();

    print_done_summary(&cfg.paths.results_dir, &written);
    progress("complete", 1, 1);
    Ok(())
}

fn print_done_summary(results_dir: &Path, written: &[PathBuf]) {
    let results_abs = pretty_path(results_dir);
    info!("================================================");
    info!("DONE. Results folder:");
    info!("  {}", results_abs.display());
    for p in written {
        info!("  • {}", p.display());
    }
    info!("================================================");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_u32_ok() {
        assert_eq!(parse_u32(b"0"), Some(0));
        assert_eq!(parse_u32(b"12345"), Some(12345));
        assert_eq!(parse_u32(b""), None);
        assert_eq!(parse_u32(b"12x"), None);
    }

    #[test]
    fn split_tabs_basic() {
        let f = split_tabs(b"a\tb\tc");
        assert_eq!(f, vec![&b"a"[..], &b"b"[..], &b"c"[..]]);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn reports_progress_for_every_pipeline_phase() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let direct = manifest.join("testdata/config.toml");
        let config_path = if direct.is_file() {
            direct
        } else {
            manifest.join("../testdata/config.toml")
        };
        let cfg = Config::load(config_path).expect("load demo config");
        let phases = Mutex::new(Vec::new());
        run_with_progress(&cfg, |phase, processed, total| {
            phases.lock().unwrap().push((phase.to_string(), processed, total));
        }).expect("run demo pipeline");
        let phases = phases.into_inner().unwrap();
        for expected in ["vertices_targets", "edges", "vertices_neighbors", "ranks", "writing", "complete"] {
            assert!(phases.iter().any(|(phase, _, _)| phase == expected), "missing progress phase: {expected}");
        }
        assert!(phases.iter().filter(|(_, _, total)| *total > 0).all(|(_, done, total)| done <= total));
    }
}
