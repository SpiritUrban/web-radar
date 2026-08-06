//! Transposed edge index — "who links **to** this domain?" in one read.
//!
//! `*-domain-edges.txt` is sorted by the *source* domain, so backlinks are
//! scattered across all 67 GB of it. Answering a single backlink question
//! therefore means reading the whole file — every time.
//!
//! This module reads it **once** and writes the transpose as CSR:
//!
//! * `inbound.off` — `node_count + 1` offsets (`u64`);
//! * `inbound.src` — every source id, grouped by destination, ascending (`u32`).
//!
//! The build is an external bucket sort, because 3.7 billion edges do not fit
//! in RAM: pass one splits edges into id-range buckets on disk, pass two sorts
//! each bucket and appends it to the CSR. Peak temp space is `8 × edges`, which
//! is why [`estimate_temp_bytes`] exists and the caller checks free space first.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use rayon::prelude::*;

use super::edges::parse_edge_line;
use super::io::{split_on_lines, trim_eol, ChunkReader};
use crate::progress::Progress;

const MAGIC: &[u8; 4] = b"WRIO";
const FORMAT: u32 = 1;
const HEADER_BYTES: usize = 64;
/// Records held in RAM per bucket while sorting; sets the bucket count.
const TARGET_BUCKET_RECORDS: u64 = 16 << 20; // 16M × 8 B = 128 MB
const MAX_BUCKETS: usize = 1024;
const MIN_BUCKETS: usize = 4;

/// Temp bytes the build needs at its peak: one 8-byte record per edge.
pub fn estimate_temp_bytes(edge_count: u64) -> u64 {
    edge_count * 8
}

/// Final size on disk: one `u32` per edge plus one `u64` per domain.
pub fn estimate_bytes(edge_count: u64, node_count: u64) -> u64 {
    edge_count * 4 + (node_count + 1) * 8 + HEADER_BYTES as u64
}

pub struct InboundBuildResult {
    pub edge_count: u64,
    pub bytes: u64,
}

/// Build the transposed index. Two passes: partition, then sort-and-append.
#[allow(clippy::too_many_arguments)]
pub fn build(
    edges: &Path,
    offsets_out: &Path,
    sources_out: &Path,
    node_capacity: u64,
    estimated_edges: u64,
    tmp_dir: &Path,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<InboundBuildResult> {
    if node_capacity == 0 {
        bail!("невідома кількість доменів — спершу побудуйте індекс пошуку");
    }
    std::fs::create_dir_all(tmp_dir)?;
    let bucket_count = pick_bucket_count(estimated_edges);
    let span = node_capacity.div_ceil(bucket_count as u64).max(1);

    let result = run_build(
        edges,
        offsets_out,
        sources_out,
        node_capacity,
        bucket_count,
        span,
        tmp_dir,
        progress,
        base,
        weight,
    );
    // Never leave 30 GB of buckets behind, cancelled or not.
    cleanup(tmp_dir, bucket_count);
    if result.is_err() {
        let _ = std::fs::remove_file(partial_path(offsets_out));
        let _ = std::fs::remove_file(partial_path(sources_out));
    }
    result
}

/// `inbound.off` → `inbound.off.part`.
///
/// `with_extension("part")` would map both `inbound.off` and `inbound.src` to
/// the same `inbound.part`, so the two writers would fight over one file.
fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

fn pick_bucket_count(estimated_edges: u64) -> usize {
    let wanted = estimated_edges.div_ceil(TARGET_BUCKET_RECORDS).max(1) as usize;
    wanted.clamp(MIN_BUCKETS, MAX_BUCKETS)
}

#[allow(clippy::too_many_arguments)]
fn run_build(
    edges: &Path,
    offsets_out: &Path,
    sources_out: &Path,
    node_capacity: u64,
    bucket_count: usize,
    span: u64,
    tmp_dir: &Path,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<InboundBuildResult> {
    // ---------------- pass 1: partition edges by destination ----------------
    let mut reader = ChunkReader::open(edges, 32 << 20)?;
    let total = reader.file_size;
    let partition_weight = weight * 0.6;
    progress.stage(
        "inbound_partition",
        format!("Читаємо зв'язки · {}", super::vertices::file_label(edges)),
        base,
        partition_weight,
        total,
    );

    let mut writers: Vec<BufWriter<File>> = Vec::with_capacity(bucket_count);
    for b in 0..bucket_count {
        writers.push(BufWriter::with_capacity(
            64 << 10,
            File::create(bucket_path(tmp_dir, b))?,
        ));
    }

    let threads = rayon::current_num_threads().max(1);
    let mut edge_count: u64 = 0;
    let mut bucket_records = vec![0u64; bucket_count];

    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        let parts = split_on_lines(chunk, threads);
        // Each worker fills its own bucket vectors; merging is a sequential
        // write, so no lock is held while parsing.
        let per_thread: Vec<Vec<Vec<u64>>> = parts
            .par_iter()
            .map(|slice| {
                let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); bucket_count];
                for_each_edge(slice, |from, to| {
                    if from == to {
                        return; // self-links are not backlinks
                    }
                    let bucket = (u64::from(to) / span) as usize;
                    if let Some(list) = buckets.get_mut(bucket) {
                        list.push(u64::from(to) << 32 | u64::from(from));
                    }
                });
                buckets
            })
            .collect();

        let mut bytes = Vec::with_capacity(1 << 16);
        for buckets in per_thread {
            for (index, records) in buckets.into_iter().enumerate() {
                if records.is_empty() {
                    continue;
                }
                edge_count += records.len() as u64;
                bucket_records[index] += records.len() as u64;
                bytes.clear();
                bytes.reserve(records.len() * 8);
                for record in &records {
                    bytes.extend_from_slice(&record.to_le_bytes());
                }
                writers[index].write_all(&bytes)?;
            }
        }
        progress.set(chunk_base + chunk.len() as u64);
    }
    for writer in &mut writers {
        writer.flush()?;
    }
    drop(writers);

    if edge_count == 0 {
        bail!(
            "у файлі edges немає жодного зв'язку виду `from_id<TAB>to_id`:\n  {}",
            edges.display()
        );
    }

    // ---------------- pass 2: sort each bucket, append to CSR ----------------
    progress.stage(
        "inbound_merge",
        "Будуємо індекс зворотних посилань",
        base + partition_weight,
        weight - partition_weight,
        edge_count,
    );

    let offsets_tmp = partial_path(offsets_out);
    let sources_tmp = partial_path(sources_out);
    if let Some(parent) = offsets_out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut offsets = BufWriter::with_capacity(4 << 20, File::create(&offsets_tmp)?);
    let mut sources = BufWriter::with_capacity(8 << 20, File::create(&sources_tmp)?);

    let mut header = [0u8; HEADER_BYTES];
    header[0..4].copy_from_slice(MAGIC);
    header[4..8].copy_from_slice(&FORMAT.to_le_bytes());
    header[8..16].copy_from_slice(&node_capacity.to_le_bytes());
    // edge_count is patched in after dedup, once the real total is known.
    offsets.write_all(&header)?;

    let mut written_sources: u64 = 0;
    let mut next_id: u64 = 0;
    let mut processed: u64 = 0;
    let mut offset_bytes: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut source_bytes: Vec<u8> = Vec::with_capacity(1 << 20);

    for bucket in 0..bucket_count {
        progress.check()?;
        let from_id = bucket as u64 * span;
        let to_id = ((bucket as u64 + 1) * span).min(node_capacity);
        if from_id >= node_capacity {
            break;
        }

        let mut records = read_bucket(&bucket_path(tmp_dir, bucket))?;
        let _ = std::fs::remove_file(bucket_path(tmp_dir, bucket));
        processed += records.len() as u64;
        // Sorting the packed `to<<32|from` sorts by destination, then source —
        // exactly the CSR order, and it makes duplicate edges adjacent.
        records.par_sort_unstable();
        records.dedup();

        offset_bytes.clear();
        source_bytes.clear();
        let mut cursor = 0usize;
        for id in from_id..to_id {
            offset_bytes.extend_from_slice(&(written_sources + (cursor as u64)).to_le_bytes());
            let key = id << 32;
            while cursor < records.len() && records[cursor] >> 32 == id {
                source_bytes
                    .extend_from_slice(&((records[cursor] & 0xFFFF_FFFF) as u32).to_le_bytes());
                cursor += 1;
            }
            let _ = key;
        }
        // Records outside [from_id, to_id) would mean a corrupt bucket.
        debug_assert_eq!(cursor, records.len());
        written_sources += cursor as u64;
        offsets.write_all(&offset_bytes)?;
        sources.write_all(&source_bytes)?;
        next_id = to_id;
        progress.set(processed);
    }

    // Trailing ids with no bucket (possible only with a truncated graph).
    while next_id < node_capacity {
        offsets.write_all(&written_sources.to_le_bytes())?;
        next_id += 1;
    }
    // Sentinel: end of the last domain's source list.
    offsets.write_all(&written_sources.to_le_bytes())?;
    offsets.flush()?;
    sources.flush()?;
    drop(offsets);
    drop(sources);

    patch_edge_count(&offsets_tmp, written_sources)?;
    std::fs::rename(&offsets_tmp, offsets_out)
        .with_context(|| format!("не вдалося зберегти {}", offsets_out.display()))?;
    std::fs::rename(&sources_tmp, sources_out)
        .with_context(|| format!("не вдалося зберегти {}", sources_out.display()))?;
    progress.finish_stage();

    log::info!(
        "inbound index: {written_sources} unique backlinks over {node_capacity} domains ({bucket_count} buckets)"
    );
    Ok(InboundBuildResult {
        edge_count: written_sources,
        bytes: estimate_bytes(written_sources, node_capacity),
    })
}

fn patch_edge_count(path: &Path, edge_count: u64) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.seek(SeekFrom::Start(16))?;
    file.write_all(&edge_count.to_le_bytes())?;
    file.flush()?;
    Ok(())
}

fn bucket_path(tmp_dir: &Path, bucket: usize) -> PathBuf {
    tmp_dir.join(format!("inbound-{bucket:04}.part"))
}

fn cleanup(tmp_dir: &Path, bucket_count: usize) {
    for bucket in 0..bucket_count {
        let _ = std::fs::remove_file(bucket_path(tmp_dir, bucket));
    }
}

fn read_bucket(path: &Path) -> Result<Vec<u64>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let len = file.metadata()?.len() as usize;
    let mut bytes = Vec::with_capacity(len);
    file.read_to_end(&mut bytes)?;
    Ok(bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap_or_default()))
        .collect())
}

fn for_each_edge(bytes: &[u8], mut visit: impl FnMut(u32, u32)) {
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
        if let Some((from, to)) = parse_edge_line(line) {
            visit(from, to);
        }
    }
}

/// Memory-mapped CSR reader.
pub struct InboundIndex {
    offsets: Mmap,
    sources: Mmap,
    pub node_capacity: u64,
    pub edge_count: u64,
}

impl InboundIndex {
    pub fn open(offsets_path: &Path, sources_path: &Path) -> Result<Self> {
        let offsets_file = File::open(offsets_path)
            .with_context(|| format!("не вдалося відкрити {}", offsets_path.display()))?;
        let sources_file = File::open(sources_path)
            .with_context(|| format!("не вдалося відкрити {}", sources_path.display()))?;
        // SAFETY: both files are written by this app and replaced atomically.
        let offsets = unsafe { Mmap::map(&offsets_file)? };
        let sources = unsafe { Mmap::map(&sources_file)? };

        if offsets.len() < HEADER_BYTES || &offsets[..4] != MAGIC {
            bail!(
                "{} не є індексом зворотних посилань",
                offsets_path.display()
            );
        }
        let format = u32::from_le_bytes(offsets[4..8].try_into()?);
        if format != FORMAT {
            bail!("індекс зворотних посилань має несумісну версію {format} — перебудуйте індекс");
        }
        let node_capacity = u64::from_le_bytes(offsets[8..16].try_into()?);
        let edge_count = u64::from_le_bytes(offsets[16..24].try_into()?);

        let needed_offsets = HEADER_BYTES as u64 + (node_capacity + 1) * 8;
        if (offsets.len() as u64) < needed_offsets {
            bail!("індекс зворотних посилань обірваний — перебудуйте індекс");
        }
        if (sources.len() as u64) < edge_count * 4 {
            bail!(
                "файл {} обірваний ({} з {} байт) — перебудуйте індекс",
                sources_path.display(),
                sources.len(),
                edge_count * 4
            );
        }
        Ok(Self {
            offsets,
            sources,
            node_capacity,
            edge_count,
        })
    }

    fn offset(&self, id: u64) -> u64 {
        let at = HEADER_BYTES + id as usize * 8;
        u64::from_le_bytes(self.offsets[at..at + 8].try_into().unwrap_or_default())
    }

    /// Number of domains linking to `id`.
    pub fn in_degree(&self, id: u32) -> u64 {
        if u64::from(id) >= self.node_capacity {
            return 0;
        }
        self.offset(u64::from(id) + 1)
            .saturating_sub(self.offset(u64::from(id)))
    }

    /// Every domain id linking to `id`, ascending, capped at `limit`.
    ///
    /// Returns `(sources, total)`; `total` is the real in-degree even when the
    /// list was capped, so the UI can say "showing 5 000 of 84 210".
    pub fn sources_of(&self, id: u32, limit: usize) -> (Vec<u32>, u64) {
        if u64::from(id) >= self.node_capacity {
            return (Vec::new(), 0);
        }
        let start = self.offset(u64::from(id));
        let end = self.offset(u64::from(id) + 1);
        let total = end.saturating_sub(start);
        let take = total.min(limit as u64) as usize;
        let from = start as usize * 4;
        let to = from + take * 4;
        if to > self.sources.len() {
            return (Vec::new(), total);
        }
        let list = self.sources[from..to]
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap_or_default()))
            .collect();
        (list, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;

    #[test]
    fn transposes_the_edge_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Edges sorted by source, as Common Crawl publishes them.
        let mut text = String::new();
        let mut expected: AHashMap<u32, Vec<u32>> = AHashMap::new();
        for from in 0..500u32 {
            for step in 0..7u32 {
                let to = (from * 13 + step * 29) % 500;
                text.push_str(&format!("{from}\t{to}\n"));
                if to != from {
                    expected.entry(to).or_default().push(from);
                }
            }
            // A duplicate edge and a self-link: both must be dropped.
            text.push_str(&format!("{from}\t{from}\n"));
            text.push_str(&format!("{from}\t{}\n", (from * 13) % 500));
        }
        let edges = dir.path().join("edges.txt");
        std::fs::write(&edges, text).expect("write edges");

        let offsets = dir.path().join("inbound.off");
        let sources = dir.path().join("inbound.src");
        let result = build(
            &edges,
            &offsets,
            &sources,
            500,
            4000,
            &dir.path().join("tmp"),
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build inbound");
        assert!(result.edge_count > 0, "index must contain edges");

        let index = InboundIndex::open(&offsets, &sources).expect("open");
        for (target, mut want) in expected {
            want.sort_unstable();
            want.dedup();
            let (got, total) = index.sources_of(target, 10_000);
            assert_eq!(got, want, "backlinks of {target}");
            assert_eq!(total, want.len() as u64);
            assert_eq!(index.in_degree(target), want.len() as u64);
        }
        // Temp buckets must not survive the build.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("tmp"))
            .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn caps_the_result_but_reports_the_true_total() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut text = String::new();
        for from in 1..200u32 {
            text.push_str(&format!("{from}\t0\n"));
        }
        let edges = dir.path().join("edges.txt");
        std::fs::write(&edges, text).expect("write");
        let offsets = dir.path().join("inbound.off");
        let sources = dir.path().join("inbound.src");
        build(
            &edges,
            &offsets,
            &sources,
            200,
            200,
            &dir.path().join("tmp"),
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build");

        let index = InboundIndex::open(&offsets, &sources).expect("open");
        let (list, total) = index.sources_of(0, 10);
        assert_eq!(list.len(), 10);
        assert_eq!(total, 199, "total must ignore the display limit");
    }
}
