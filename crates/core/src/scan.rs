//! Streaming fallback: answer without an index by reading all three files.
//!
//! This is the original Web Radar pipeline and it stays for three reasons:
//! it works on compressed sources, it needs no extra disk, and it is the only
//! thing that runs before an index exists. It costs ~79 GB of reads per run for
//! the 2026 apr–jun graph, which is precisely what [`crate::index`] removes.
//!
//! Pipeline:
//! 1. scan **vertices** → target reverse-domains to node ids;
//! 2. stream **edges** → inbound *and* outbound neighbours of every target;
//! 3. re-scan **vertices** → neighbour ids back to domain names;
//! 4. scan **ranks** → ranks for targets and neighbours;
//! 5. write `results/{reversed-domain}.json` per target.

use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};
use log::{info, warn};

use crate::config::{Config, RankMetric, ResolvedTarget};
use crate::index::io::{parse_f64, parse_u32, tab_fields, trim_eol, ChunkReader};
use crate::model::{LinkEntry, TargetResult};
use crate::progress::Progress;
use crate::reverse::{from_reverse, reverse_to_filename};

/// Inbound + outbound adjacency collected in one edges pass.
struct EdgeSets {
    /// target_id → ids that link to it
    inbound: AHashMap<u32, AHashSet<u32>>,
    /// target_id → ids it links to
    outbound: AHashMap<u32, AHashSet<u32>>,
}

/// Run the full multi-pass pipeline, returning the files written.
pub fn run(cfg: &Config, progress: &Progress) -> Result<Vec<PathBuf>> {
    cfg.ensure_files()?;
    let targets = cfg.resolved_targets()?;
    info!(
        "scanning for {} target(s), metric {}",
        targets.len(),
        cfg.rank_metric.as_str()
    );

    // Weight the phases by the bytes each one actually reads, so the bar does
    // not sit at 20 % for half an hour and then jump.
    let vertices_bytes = size_of(&cfg.paths.vertices);
    let edges_bytes = size_of(&cfg.paths.edges);
    let ranks_bytes = size_of(&cfg.paths.ranks);
    let total = (vertices_bytes * 2 + edges_bytes + ranks_bytes).max(1) as f64;
    let weight = |bytes: u64| bytes as f64 / total;

    let mut base = 0.0;
    let target_ids = pass_resolve_targets(
        &cfg.paths.vertices,
        &targets,
        progress,
        base,
        weight(vertices_bytes),
    )?;
    base += weight(vertices_bytes);
    let found_revs: AHashSet<&str> = target_ids.values().map(String::as_str).collect();

    // Always materialise not-found stubs so missing domains are visible.
    let mut written = write_not_found_stubs(&cfg.paths.results_dir, &targets, &found_revs)?;
    if target_ids.is_empty() {
        warn!("none of the configured targets exist in the vertices graph");
        return Ok(written);
    }

    let edges = pass_collect_edges(
        &cfg.paths.edges,
        &target_ids,
        progress,
        base,
        weight(edges_bytes),
    )?;
    base += weight(edges_bytes);

    let mut needed_ids: AHashSet<u32> = AHashSet::new();
    for set in edges.inbound.values().chain(edges.outbound.values()) {
        needed_ids.extend(set.iter().copied());
    }
    info!("neighbour domains to resolve: {}", needed_ids.len());

    let id_to_rev = pass_resolve_names(
        &cfg.paths.vertices,
        &needed_ids,
        progress,
        base,
        weight(vertices_bytes),
    )?;
    base += weight(vertices_bytes);

    let mut needed_revs: AHashSet<&str> = id_to_rev.values().map(String::as_str).collect();
    for rev in target_ids.values() {
        needed_revs.insert(rev.as_str());
    }
    let ranks = pass_load_ranks(
        &cfg.paths.ranks,
        &needed_revs,
        cfg.rank_metric,
        progress,
        base,
        weight(ranks_bytes),
    )?;

    progress.stage("writing", "Записуємо результати", 0.99, 0.01, 1);
    written.extend(write_found_results(
        &cfg.paths.results_dir,
        &target_ids,
        &edges,
        &id_to_rev,
        &ranks,
        cfg.rank_metric,
    )?);
    written.sort();
    progress.finish_stage();
    Ok(written)
}

fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Vertices line: `id \t reverse_domain \t n_hosts`
fn pass_resolve_targets(
    vertices: &Path,
    targets: &[ResolvedTarget],
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<AHashMap<u32, String>> {
    let wanted: AHashSet<&str> = targets.iter().map(|t| t.reverse.as_str()).collect();
    let mut reader = ChunkReader::open(vertices, 8 << 20)?;
    progress.stage(
        "vertices_targets",
        "Шукаємо цільові домени у vertices",
        base,
        weight,
        reader.file_size,
    );

    let mut id_to_rev: AHashMap<u32, String> = AHashMap::with_capacity(wanted.len());
    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        for_each_vertex(chunk, |id, name| {
            if wanted.contains(name) {
                id_to_rev.insert(id, name.to_owned());
            }
        });
        progress.set(chunk_base + chunk.len() as u64);
        if id_to_rev.len() == wanted.len() {
            break; // every target found; the rest of the file cannot add more
        }
    }
    progress.finish_stage();

    for target in targets {
        if !id_to_rev.values().any(|rev| rev == &target.reverse) {
            warn!(
                "target '{}' (rev '{}') is absent from the vertices file — writing a found=false stub",
                target.domain, target.reverse
            );
        }
    }
    Ok(id_to_rev)
}

/// Edges line: `from_id \t to_id`
fn pass_collect_edges(
    edges: &Path,
    target_ids: &AHashMap<u32, String>,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<EdgeSets> {
    let mut inbound: AHashMap<u32, AHashSet<u32>> =
        target_ids.keys().map(|&id| (id, AHashSet::new())).collect();
    let mut outbound: AHashMap<u32, AHashSet<u32>> =
        target_ids.keys().map(|&id| (id, AHashSet::new())).collect();

    let mut reader = ChunkReader::open(edges, 32 << 20)?;
    progress.stage(
        "edges",
        "Скануємо зв'язки у великому файлі edges",
        base,
        weight,
        reader.file_size,
    );

    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        let mut cursor = 0usize;
        while cursor < chunk.len() {
            let end = memchr::memchr(b'\n', &chunk[cursor..])
                .map(|p| cursor + p + 1)
                .unwrap_or(chunk.len());
            let line = trim_eol(&chunk[cursor..end]);
            cursor = end;
            if line.is_empty() || line[0] == b'#' {
                continue;
            }
            let Some((from, to)) = crate::index::edges::parse_edge_line(line) else {
                continue;
            };
            if from == to {
                continue;
            }
            if let Some(sources) = inbound.get_mut(&to) {
                sources.insert(from);
            }
            if let Some(destinations) = outbound.get_mut(&from) {
                destinations.insert(to);
            }
        }
        progress.set(chunk_base + chunk.len() as u64);
    }
    progress.finish_stage();

    for (&id, rev) in target_ids {
        info!(
            "  {rev}: {} inbound, {} outbound",
            inbound.get(&id).map(|set| set.len()).unwrap_or(0),
            outbound.get(&id).map(|set| set.len()).unwrap_or(0)
        );
    }
    Ok(EdgeSets { inbound, outbound })
}

fn pass_resolve_names(
    vertices: &Path,
    needed: &AHashSet<u32>,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<AHashMap<u32, String>> {
    if needed.is_empty() {
        return Ok(AHashMap::new());
    }
    let mut reader = ChunkReader::open(vertices, 8 << 20)?;
    progress.stage(
        "vertices_neighbors",
        "Визначаємо назви сусідніх доменів",
        base,
        weight,
        reader.file_size,
    );

    let mut id_to_rev: AHashMap<u32, String> = AHashMap::with_capacity(needed.len());
    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        for_each_vertex(chunk, |id, name| {
            if needed.contains(&id) {
                id_to_rev.insert(id, name.to_owned());
            }
        });
        progress.set(chunk_base + chunk.len() as u64);
        if id_to_rev.len() == needed.len() {
            break;
        }
    }
    progress.finish_stage();

    if id_to_rev.len() < needed.len() {
        warn!(
            "{} neighbour id(s) had no name in the vertices file",
            needed.len() - id_to_rev.len()
        );
    }
    Ok(id_to_rev)
}

/// Ranks line: `hc_pos \t hc_val \t pr_pos \t pr_val \t host_rev [\t n_hosts]`
fn pass_load_ranks(
    ranks_path: &Path,
    needed: &AHashSet<&str>,
    metric: RankMetric,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<AHashMap<String, f64>> {
    if needed.is_empty() {
        return Ok(AHashMap::new());
    }
    let mut reader = ChunkReader::open(ranks_path, 8 << 20)?;
    progress.stage(
        "ranks",
        "Завантажуємо рейтинги доменів",
        base,
        weight,
        reader.file_size,
    );

    let (mut harmonic_col, mut pagerank_col, mut host_col) = (1usize, 3usize, 4usize);
    let mut header_seen = false;
    let mut ranks: AHashMap<String, f64> = AHashMap::with_capacity(needed.len());
    let mut field_ends: Vec<usize> = Vec::with_capacity(8);

    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        let mut cursor = 0usize;
        while cursor < chunk.len() {
            let end = memchr::memchr(b'\n', &chunk[cursor..])
                .map(|p| cursor + p + 1)
                .unwrap_or(chunk.len());
            let line = trim_eol(&chunk[cursor..end]);
            cursor = end;
            if line.is_empty() {
                continue;
            }
            if line[0] == b'#' {
                if !header_seen {
                    header_seen = true;
                    if let Ok(header) = std::str::from_utf8(line) {
                        parse_ranks_header(
                            header,
                            &mut harmonic_col,
                            &mut pagerank_col,
                            &mut host_col,
                        );
                    }
                }
                continue;
            }

            tab_fields(line, &mut field_ends);
            let highest = host_col.max(pagerank_col).max(harmonic_col);
            if field_ends.len() < highest {
                continue;
            }
            let field = |i: usize| -> &[u8] {
                let start = if i == 0 { 0 } else { field_ends[i - 1] + 1 };
                let stop = field_ends.get(i).copied().unwrap_or(line.len());
                &line[start..stop]
            };
            let Ok(host) = std::str::from_utf8(field(host_col)) else {
                continue;
            };
            if !needed.contains(host) {
                continue;
            }
            let column = match metric {
                RankMetric::Pagerank => pagerank_col,
                RankMetric::Harmonic => harmonic_col,
            };
            if let Some(value) = parse_f64(field(column)) {
                ranks.insert(host.to_owned(), value);
            }
            if ranks.len() == needed.len() {
                break;
            }
        }
        progress.set(chunk_base + chunk.len() as u64);
        if ranks.len() == needed.len() {
            break;
        }
    }
    progress.finish_stage();

    if ranks.len() < needed.len() {
        warn!(
            "{} domain(s) had no rank row (using 0.0)",
            needed.len() - ranks.len()
        );
    }
    Ok(ranks)
}

fn parse_ranks_header(
    header: &str,
    harmonic_col: &mut usize,
    pagerank_col: &mut usize,
    host_col: &mut usize,
) {
    for (i, column) in header.trim_start_matches('#').split('\t').enumerate() {
        match column
            .trim()
            .trim_start_matches('#')
            .to_ascii_lowercase()
            .as_str()
        {
            "harmonicc_val" | "harmonic_val" | "harmonic" => *harmonic_col = i,
            "pr_val" | "pagerank" | "pr" => *pagerank_col = i,
            "host_rev" | "domain_rev" | "rev_host" | "rev_domain" => *host_col = i,
            _ => {}
        }
    }
}

fn for_each_vertex(bytes: &[u8], mut visit: impl FnMut(u32, &str)) {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let end = memchr::memchr(b'\n', &bytes[cursor..])
            .map(|p| cursor + p + 1)
            .unwrap_or(bytes.len());
        let line = trim_eol(&bytes[cursor..end]);
        cursor = end;
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let Some(tab) = memchr::memchr(b'\t', line) else {
            continue;
        };
        let Some(id) = parse_u32(&line[..tab]) else {
            continue;
        };
        let rest = &line[tab + 1..];
        let name_end = memchr::memchr(b'\t', rest).unwrap_or(rest.len());
        if let Ok(name) = std::str::from_utf8(&rest[..name_end]) {
            if !name.is_empty() {
                visit(id, name);
            }
        }
    }
}

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
                .filter_map(|id| {
                    let rev = id_to_rev.get(id)?;
                    Some(LinkEntry::new(
                        from_reverse(rev),
                        ranks.get(rev).copied().unwrap_or(0.0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    sort_links(&mut entries);
    entries
}

fn write_json_result(
    results_dir: &Path,
    rev_target: &str,
    result: &TargetResult,
) -> Result<PathBuf> {
    let path = results_dir.join(reverse_to_filename(rev_target) + ".json");
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&path, json + "\n")
        .with_context(|| format!("не вдалося записати {}", path.display()))?;
    Ok(pretty_path(&path))
}

/// Write `found: false` stubs for targets missing from the vertices graph.
fn write_not_found_stubs(
    results_dir: &Path,
    targets: &[ResolvedTarget],
    found: &AHashSet<&str>,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(results_dir)
        .with_context(|| format!("не вдалося створити {}", results_dir.display()))?;

    let mut missing: Vec<&ResolvedTarget> = targets
        .iter()
        .filter(|target| !found.contains(target.reverse.as_str()))
        .collect();
    missing.sort_by(|a, b| a.reverse.cmp(&b.reverse));

    let mut written = Vec::new();
    for target in missing {
        let mut result = TargetResult::not_found(&target.domain);
        result.source = Some("scan".into());
        written.push(write_json_result(results_dir, &target.reverse, &result)?);
    }
    Ok(written)
}

fn write_found_results(
    results_dir: &Path,
    target_ids: &AHashMap<u32, String>,
    edges: &EdgeSets,
    id_to_rev: &AHashMap<u32, String>,
    ranks: &AHashMap<String, f64>,
    metric: RankMetric,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(results_dir)
        .with_context(|| format!("не вдалося створити {}", results_dir.display()))?;

    // Stable order by reverse domain so multi-target runs are predictable.
    let mut ordered: Vec<(&u32, &String)> = target_ids.iter().collect();
    ordered.sort_by(|a, b| a.1.cmp(b.1));

    let mut written = Vec::new();
    for (&target_id, rev_target) in ordered {
        let inbound = links_from_ids(edges.inbound.get(&target_id), id_to_rev, ranks);
        let outbound = links_from_ids(edges.outbound.get(&target_id), id_to_rev, ranks);
        let result = TargetResult {
            domain: from_reverse(rev_target),
            found: true,
            rank: Some(ranks.get(rev_target.as_str()).copied().unwrap_or(0.0)),
            metric: Some(metric.as_str().into()),
            node_id: Some(target_id),
            position: None,
            inbound_total: Some(inbound.len()),
            outbound_total: Some(outbound.len()),
            source: Some("scan".into()),
            inbound,
            outbound,
        };
        let path = write_json_result(results_dir, rev_target, &result)?;
        info!(
            "wrote {} — {} inbound, {} outbound",
            path.display(),
            result.inbound.len(),
            result.outbound.len()
        );
        written.push(path);
    }
    Ok(written)
}

/// Absolute path without the Windows `\\?\` prefix (nicer logs).
fn pretty_path(path: &Path) -> PathBuf {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = absolute.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => absolute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetResult;

    fn demo_config(dir: &Path) -> Config {
        std::fs::write(
            dir.join("vertices.txt"),
            "0\tcom.aaa\t1\n1\tcom.bbb\t1\n2\torg.ccc\t1\n",
        )
        .expect("vertices");
        std::fs::write(dir.join("edges.txt"), "0\t2\n1\t0\n2\t0\n").expect("edges");
        std::fs::write(
            dir.join("ranks.txt"),
            "#harmonicc_pos\t#harmonicc_val\t#pr_pos\t#pr_val\t#host_rev\t#n_hosts\n\
             1\t900.0\t1\t5.0E-9\tcom.aaa\t1\n\
             2\t800.0\t3\t3.0E-9\tcom.bbb\t1\n\
             3\t700.0\t2\t4.0E-9\torg.ccc\t1\n",
        )
        .expect("ranks");

        let toml = r#"
            [paths]
            vertices = "vertices.txt"
            edges = "edges.txt"
            ranks = "ranks.txt"

            [[targets]]
            domain = "https://aaa.com/"

            [[targets]]
            domain = "absent.example"
        "#;
        let mut cfg: Config = toml::from_str(toml).expect("parse");
        cfg.resolve_relative_to(dir);
        cfg
    }

    #[test]
    fn produces_the_same_answer_as_the_index_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = demo_config(dir.path());
        let written = run(&cfg, &Progress::silent()).expect("scan");
        assert_eq!(written.len(), 2, "one file per target: {written:?}");

        let raw =
            std::fs::read_to_string(cfg.paths.results_dir.join("com.aaa.json")).expect("read");
        let result: TargetResult = serde_json::from_str(&raw).expect("parse");
        assert!(result.found);
        assert_eq!(result.rank, Some(5.0e-9));
        let inbound: Vec<&str> = result.inbound.iter().map(|e| e.domain.as_str()).collect();
        assert_eq!(inbound, vec!["ccc.org", "bbb.com"]);
        let outbound: Vec<&str> = result.outbound.iter().map(|e| e.domain.as_str()).collect();
        assert_eq!(outbound, vec!["ccc.org"]);

        let raw = std::fs::read_to_string(cfg.paths.results_dir.join("example.absent.json"))
            .expect("stub for the missing domain");
        let missing: TargetResult = serde_json::from_str(&raw).expect("parse");
        assert!(
            !missing.found,
            "an absent domain must be recorded, not skipped"
        );
    }

    #[test]
    fn reports_every_pipeline_phase_and_finishes_at_one() {
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = demo_config(dir.path());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let progress = Progress::new(Arc::new(AtomicBool::new(false)), move |update| {
            sink.lock().unwrap().push((update.stage, update.overall));
        });

        run(&cfg, &progress).expect("scan");
        let seen = seen.lock().unwrap().clone();
        for expected in [
            "vertices_targets",
            "edges",
            "vertices_neighbors",
            "ranks",
            "writing",
        ] {
            assert!(
                seen.iter().any(|(stage, _)| stage == expected),
                "missing phase {expected} in {seen:?}"
            );
        }
        assert!(seen.iter().all(|(_, overall)| *overall <= 1.0));
    }

    #[test]
    fn cancellation_stops_the_run() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = demo_config(dir.path());
        let cancel = Arc::new(AtomicBool::new(true));
        let progress = Progress::new(Arc::clone(&cancel), |_| {});
        cancel.store(true, Ordering::Relaxed);

        let error = run(&cfg, &progress).expect_err("cancelled run must fail");
        assert!(
            format!("{error}").contains("скасовано"),
            "unexpected: {error}"
        );
    }
}
