//! Rank lookup by node id — turns `*-domain-ranks.txt` into a flat array.
//!
//! The ranks file is sorted by harmonic-centrality position, not by domain, so
//! unlike vertices and edges it cannot be binary searched. It is joined against
//! the vertex index once and stored as `node_count × 16` bytes
//! (`pagerank`, `harmonic`, and both positions), which makes every later rank
//! lookup a single indexed read.
//!
//! The join is done in id-range partitions so neither side is ever fully in RAM:
//! pass one splits the ranks rows into buckets by where their domain sits in the
//! vertices file, pass two walks the vertices file once, one contiguous slice
//! per bucket.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use memmap2::Mmap;

use super::io::{parse_f64, parse_u32, trim_eol, ChunkReader};
use super::vertices::VertexIndex;
use crate::config::RankMetric;
use crate::progress::Progress;

const MAGIC: &[u8; 4] = b"WRRK";
const FORMAT: u32 = 1;
const HEADER_BYTES: usize = 64;
pub const ROW_BYTES: usize = 16;
/// How many id-range partitions the join uses. 256 keeps each side's working
/// set around a hundred megabytes for the full 121-million-domain graph.
const PARTITIONS: usize = 256;

/// Rank of one domain, as stored in the index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankRow {
    pub pagerank: f32,
    pub harmonic: f32,
    /// 1 = highest PageRank in the whole graph; 0 = unknown.
    pub pagerank_position: u32,
    pub harmonic_position: u32,
}

impl RankRow {
    pub fn value(&self, metric: RankMetric) -> f64 {
        match metric {
            RankMetric::Pagerank => f64::from(self.pagerank),
            RankMetric::Harmonic => f64::from(self.harmonic),
        }
    }

    pub fn position(&self, metric: RankMetric) -> Option<u32> {
        let position = match metric {
            RankMetric::Pagerank => self.pagerank_position,
            RankMetric::Harmonic => self.harmonic_position,
        };
        (position > 0).then_some(position)
    }

    fn write_to(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&self.pagerank.to_le_bytes());
        out[4..8].copy_from_slice(&self.harmonic.to_le_bytes());
        out[8..12].copy_from_slice(&self.pagerank_position.to_le_bytes());
        out[12..16].copy_from_slice(&self.harmonic_position.to_le_bytes());
    }
}

/// Column layout of the ranks file, detected from its `#`-header.
#[derive(Debug, Clone, Copy)]
struct Columns {
    harmonic_position: usize,
    harmonic_value: usize,
    pagerank_position: usize,
    pagerank_value: usize,
    host_rev: usize,
}

impl Default for Columns {
    /// Common Crawl's `join_ranks` output order.
    fn default() -> Self {
        Self {
            harmonic_position: 0,
            harmonic_value: 1,
            pagerank_position: 2,
            pagerank_value: 3,
            host_rev: 4,
        }
    }
}

impl Columns {
    fn from_header(header: &str) -> Self {
        let mut columns = Self::default();
        for (i, name) in header.trim_start_matches('#').split('\t').enumerate() {
            match name
                .trim()
                .trim_start_matches('#')
                .to_ascii_lowercase()
                .as_str()
            {
                "harmonicc_pos" | "harmonic_pos" => columns.harmonic_position = i,
                "harmonicc_val" | "harmonic_val" | "harmonic" => columns.harmonic_value = i,
                "pr_pos" | "pagerank_pos" => columns.pagerank_position = i,
                "pr_val" | "pagerank" | "pr" => columns.pagerank_value = i,
                "host_rev" | "domain_rev" | "rev_host" | "rev_domain" => columns.host_rev = i,
                _ => {}
            }
        }
        columns
    }

    fn max(&self) -> usize {
        self.harmonic_position
            .max(self.harmonic_value)
            .max(self.pagerank_position)
            .max(self.pagerank_value)
            .max(self.host_rev)
    }
}

/// Build `ranks.bin` by joining the ranks file against the vertex index.
#[allow(clippy::too_many_arguments)]
pub fn build(
    source: &Path,
    out: &Path,
    vertices: &VertexIndex,
    node_capacity: u64,
    tmp_dir: &Path,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<u64> {
    if !vertices.sorted_by_name {
        bail!("індекс рейтингів потребує vertices, відсортованого за доменом");
    }
    std::fs::create_dir_all(tmp_dir)?;
    let blocks_per_partition = vertices.block_count().div_ceil(PARTITIONS).max(1);
    let partition_count = vertices.block_count().div_ceil(blocks_per_partition);

    // ---- pass 1: split ranks rows by where their domain lives in vertices ----
    let mut reader = ChunkReader::open(source, 8 << 20)?;
    let total = reader.file_size;
    let split = weight * 0.55;
    progress.stage(
        "ranks_partition",
        format!("Читаємо рейтинги · {}", super::vertices::file_label(source)),
        base,
        split,
        total,
    );

    let mut writers: Vec<BufWriter<File>> = Vec::with_capacity(partition_count);
    for p in 0..partition_count {
        let file = File::create(tmp_dir.join(format!("ranks-{p:04}.part")))?;
        writers.push(BufWriter::with_capacity(64 << 10, file));
    }

    let mut columns = Columns::default();
    let mut header_seen = false;
    let mut rows: u64 = 0;
    let mut field_ends: Vec<usize> = Vec::with_capacity(8);
    let mut record = Vec::with_capacity(64);

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
                    if let Ok(text) = std::str::from_utf8(line) {
                        columns = Columns::from_header(text);
                    }
                }
                continue;
            }

            super::io::tab_fields(line, &mut field_ends);
            if field_ends.len() < columns.max() {
                continue;
            }
            let field = |i: usize| -> &[u8] {
                let start = if i == 0 { 0 } else { field_ends[i - 1] + 1 };
                let stop = field_ends.get(i).copied().unwrap_or(line.len());
                &line[start..stop]
            };
            let name = field(columns.host_rev);
            if name.is_empty() {
                continue;
            }
            let row = RankRow {
                pagerank: parse_f64(field(columns.pagerank_value)).unwrap_or(0.0) as f32,
                harmonic: parse_f64(field(columns.harmonic_value)).unwrap_or(0.0) as f32,
                pagerank_position: parse_u32(field(columns.pagerank_position)).unwrap_or(0),
                harmonic_position: parse_u32(field(columns.harmonic_position)).unwrap_or(0),
            };

            let block = vertices.block_of_name(name);
            let partition = (block / blocks_per_partition).min(partition_count - 1);

            record.clear();
            record.push(name.len().min(255) as u8);
            record.extend_from_slice(&name[..name.len().min(255)]);
            let mut bytes = [0u8; ROW_BYTES];
            row.write_to(&mut bytes);
            record.extend_from_slice(&bytes);
            writers[partition].write_all(&record)?;
            rows += 1;
        }
        progress.set(chunk_base + chunk.len() as u64);
    }
    for writer in &mut writers {
        writer.flush()?;
    }
    drop(writers);

    if rows == 0 {
        cleanup(tmp_dir, partition_count);
        bail!(
            "у файлі ranks не знайдено жодного рядка з доменом:\n  {}\n\
             Очікується формат Common Crawl: `#harmonicc_pos<TAB>#harmonicc_val<TAB>#pr_pos<TAB>#pr_val<TAB>#host_rev`",
            source.display()
        );
    }

    // ---- pass 2: walk vertices once, one contiguous slice per partition ----
    progress.stage(
        "ranks_merge",
        "Зіставляємо рейтинги з доменами",
        base + split,
        weight - split,
        partition_count as u64,
    );

    let tmp_out = out.with_extension("part");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::with_capacity(4 << 20, File::create(&tmp_out)?);
    write_header(&mut writer, node_capacity)?;

    let mut matched: u64 = 0;
    let mut next_id: u64 = 0;
    for partition in 0..partition_count {
        progress.check()?;
        let first_block = partition * blocks_per_partition;
        let last_block = ((partition + 1) * blocks_per_partition).min(vertices.block_count());
        let from_id = u64::from(vertices.block_first_id(first_block));
        let to_id = if last_block >= vertices.block_count() {
            node_capacity
        } else {
            u64::from(vertices.block_first_id(last_block))
        };

        // Ids below the partition start were never covered (gaps in the file).
        if from_id > next_id {
            write_zeros(&mut writer, (from_id - next_id) as usize)?;
        }
        let span = to_id.saturating_sub(from_id) as usize;
        let mut slab = vec![0u8; span * ROW_BYTES];

        let names = vertices.names_in_blocks(first_block, last_block)?;
        let path = tmp_dir.join(format!("ranks-{partition:04}.part"));
        let mut bucket = Vec::new();
        File::open(&path)?.read_to_end(&mut bucket)?;

        let mut at = 0usize;
        while at < bucket.len() {
            let len = bucket[at] as usize;
            let name_start = at + 1;
            let value_start = name_start + len;
            if value_start + ROW_BYTES > bucket.len() {
                break;
            }
            let name = &bucket[name_start..value_start];
            if let Some(&id) = std::str::from_utf8(name).ok().and_then(|n| names.get(n)) {
                let slot = u64::from(id).saturating_sub(from_id) as usize;
                if slot < span {
                    slab[slot * ROW_BYTES..(slot + 1) * ROW_BYTES]
                        .copy_from_slice(&bucket[value_start..value_start + ROW_BYTES]);
                    matched += 1;
                }
            }
            at = value_start + ROW_BYTES;
        }
        drop(bucket);
        let _ = std::fs::remove_file(&path);

        writer.write_all(&slab)?;
        next_id = to_id;
        progress.set(partition as u64 + 1);
    }
    if next_id < node_capacity {
        write_zeros(&mut writer, (node_capacity - next_id) as usize)?;
    }
    writer.flush()?;
    drop(writer);
    cleanup(tmp_dir, partition_count);

    if matched == 0 {
        let _ = std::fs::remove_file(&tmp_out);
        bail!(
            "жоден домен із файлу ranks не знайдено у vertices — схоже, файли з різних випусків Common Crawl"
        );
    }
    std::fs::rename(&tmp_out, out)
        .with_context(|| format!("не вдалося зберегти {}", out.display()))?;
    progress.finish_stage();
    log::info!("ranks index: {matched} of {rows} rows matched a domain id");
    Ok(matched)
}

fn write_header(out: &mut impl Write, node_capacity: u64) -> Result<()> {
    let mut header = [0u8; HEADER_BYTES];
    header[0..4].copy_from_slice(MAGIC);
    header[4..8].copy_from_slice(&FORMAT.to_le_bytes());
    header[8..16].copy_from_slice(&node_capacity.to_le_bytes());
    out.write_all(&header)?;
    Ok(())
}

fn write_zeros(out: &mut impl Write, rows: usize) -> Result<()> {
    let zeros = vec![0u8; 64 * ROW_BYTES];
    let mut left = rows;
    while left > 0 {
        let batch = left.min(64);
        out.write_all(&zeros[..batch * ROW_BYTES])?;
        left -= batch;
    }
    Ok(())
}

fn cleanup(tmp_dir: &Path, partitions: usize) {
    for p in 0..partitions {
        let _ = std::fs::remove_file(tmp_dir.join(format!("ranks-{p:04}.part")));
    }
}

/// Memory-mapped `ranks.bin`.
pub struct RankIndex {
    map: Mmap,
    pub node_capacity: u64,
}

impl RankIndex {
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("не вдалося відкрити {}", path.display()))?;
        // SAFETY: the index files are written by this app and never truncated
        // while open; a stale one fails the header/length check below.
        let map = unsafe { Mmap::map(&file)? };
        if map.len() < HEADER_BYTES || &map[..4] != MAGIC {
            bail!("{} не є індексом рейтингів", path.display());
        }
        let format = u32::from_le_bytes(map[4..8].try_into()?);
        if format != FORMAT {
            bail!("індекс рейтингів має несумісну версію {format} — перебудуйте індекс");
        }
        let node_capacity = u64::from_le_bytes(map[8..16].try_into()?);
        let expected = HEADER_BYTES as u64 + node_capacity * ROW_BYTES as u64;
        if (map.len() as u64) < expected {
            bail!(
                "індекс рейтингів обірваний ({} з {expected} байт) — перебудуйте індекс",
                map.len()
            );
        }
        Ok(Self { map, node_capacity })
    }

    pub fn get(&self, id: u32) -> Option<RankRow> {
        if u64::from(id) >= self.node_capacity {
            return None;
        }
        let at = HEADER_BYTES + id as usize * ROW_BYTES;
        let row = &self.map[at..at + ROW_BYTES];
        Some(RankRow {
            pagerank: f32::from_le_bytes(row[0..4].try_into().ok()?),
            harmonic: f32::from_le_bytes(row[4..8].try_into().ok()?),
            pagerank_position: u32::from_le_bytes(row[8..12].try_into().ok()?),
            harmonic_position: u32::from_le_bytes(row[12..16].try_into().ok()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::vertices;

    #[test]
    fn joins_ranks_onto_domain_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut vertex_text = String::new();
        for i in 0..1000u32 {
            vertex_text.push_str(&format!("{i}\tcom.site{i:04}\t1\n"));
        }
        let vertices_path = dir.path().join("vertices.txt");
        std::fs::write(&vertices_path, vertex_text).expect("write vertices");
        let vertices_index = dir.path().join("vertices.idx");
        let summary = vertices::build(
            &vertices_path,
            &vertices_index,
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build vertices");
        let index = vertices::VertexIndex::open(&vertices_index, &vertices_path).expect("open");

        // Ranks in a completely different order, as in the real dump.
        let mut ranks_text =
            String::from("#harmonicc_pos\t#harmonicc_val\t#pr_pos\t#pr_val\t#host_rev\t#n_hosts\n");
        for i in (0..1000u32).rev() {
            let position = 1000 - i;
            ranks_text.push_str(&format!(
                "{position}\t{}.0\t{position}\t{}E-9\tcom.site{i:04}\t1\n",
                1_000_000 - i,
                4 + i % 5
            ));
        }
        let ranks_path = dir.path().join("ranks.txt");
        std::fs::write(&ranks_path, ranks_text).expect("write ranks");

        let out = dir.path().join("ranks.bin");
        let matched = build(
            &ranks_path,
            &out,
            &index,
            u64::from(summary.max_id) + 1,
            &dir.path().join("tmp"),
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build ranks");
        assert_eq!(matched, 1000, "every domain must find its rank row");

        let ranks = RankIndex::open(&out).expect("open ranks");
        let row = ranks.get(7).expect("rank of id 7");
        assert!(
            (row.pagerank - 6e-9).abs() < 1e-12,
            "unexpected pagerank {row:?}"
        );
        assert_eq!(row.harmonic_position, 1000 - 7);
        assert_eq!(ranks.get(999).expect("last").harmonic_position, 1);
        assert!(ranks.get(5000).is_none(), "out-of-range id must be None");
    }

    #[test]
    fn detects_reordered_header_columns() {
        let columns = Columns::from_header("#host_rev\t#pr_val\t#harmonicc_val");
        assert_eq!(columns.host_rev, 0);
        assert_eq!(columns.pagerank_value, 1);
        assert_eq!(columns.harmonic_value, 2);
    }
}
