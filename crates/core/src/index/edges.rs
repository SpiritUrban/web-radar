//! Sparse block index over `*-domain-edges.txt` for **outbound** lookups.
//!
//! The file is `from_id \t to_id`, sorted by `from_id` (then by `to_id`), so all
//! edges of one domain sit in a contiguous run. Finding that run needs nothing
//! but the id at a few thousand file positions — which is why this index is
//! built by *sampling* the file every [`BLOCK_BYTES`], not by reading it.
//!
//! For a 67 GB edges file that is ~16 000 four-kilobyte reads (a few seconds)
//! and a ~256 KB index, after which "where does this domain link to?" is one
//! binary search and one short read.
//!
//! The other direction — "who links **to** this domain?" — cannot be answered
//! this way and is what [`super::inbound`] exists for.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::io::{parse_u32, read_at, read_range, trim_eol};
use crate::progress::Progress;

const MAGIC: &[u8; 4] = b"WREI";
const FORMAT: u32 = 1;
/// Bytes between samples. 4 MB → ~16 000 entries for a 67 GB file.
pub const BLOCK_BYTES: u64 = 4 << 20;
const HEADER_BYTES: usize = 48;
const ENTRY_BYTES: usize = 16;
/// Enough to hold a full line even with absurd ids.
const PROBE_BYTES: usize = 4096;

struct BlockEntry {
    offset: u64,
    first_from: u32,
}

pub struct EdgeIndex {
    file: File,
    blocks: Vec<BlockEntry>,
    pub file_size: u64,
    pub sorted_by_from: bool,
    /// Estimated from the average sampled line length — exact counts need a
    /// full pass, which the inbound build does anyway.
    pub estimated_edges: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct EdgesSummary {
    pub sorted_by_from: bool,
    pub estimated_edges: u64,
    pub max_from_id: u32,
}

/// Build the sampled block index. Reads ~4 KB per 4 MB of the source.
pub fn build(
    source: &Path,
    out: &Path,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<EdgesSummary> {
    build_with_block_size(source, out, BLOCK_BYTES, progress, base, weight)
}

/// Same, with the sampling interval spelled out — tests use a small one so a
/// fixture of a few hundred kilobytes still exercises multi-block lookups.
pub(crate) fn build_with_block_size(
    source: &Path,
    out: &Path,
    block_bytes: u64,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<EdgesSummary> {
    let file =
        File::open(source).with_context(|| format!("не вдалося відкрити {}", source.display()))?;
    let file_size = file.metadata()?.len();
    if file_size == 0 {
        bail!("файл edges порожній:\n  {}", source.display());
    }

    progress.stage(
        "edges_index",
        format!(
            "Розмічаємо зв'язки · {}",
            super::vertices::file_label(source)
        ),
        base,
        weight,
        file_size,
    );

    let mut blocks: Vec<BlockEntry> = Vec::new();
    let mut sorted = true;
    let mut previous: Option<u32> = None;
    let mut max_from_id = 0u32;
    let mut sampled_lines = 0u64;
    let mut sampled_bytes = 0u64;

    let mut position = 0u64;
    let mut probe = vec![0u8; PROBE_BYTES];
    while position < file_size {
        progress.check()?;
        let read = read_at(&file, position, &mut probe)?;
        if read == 0 {
            break;
        }
        let window = &probe[..read];
        // At offset 0 the file starts on a line boundary; elsewhere skip the
        // partial line we landed in the middle of.
        let start = if position == 0 {
            0
        } else {
            match memchr::memchr(b'\n', window) {
                Some(pos) => pos + 1,
                None => {
                    position += read as u64;
                    continue;
                }
            }
        };
        let rest = &window[start..];
        let line_end = memchr::memchr(b'\n', rest).unwrap_or(rest.len());
        let line = trim_eol(&rest[..line_end]);
        if let Some((from, _to)) = parse_edge_line(line) {
            if previous.is_some_and(|p| from < p) {
                sorted = false;
            }
            previous = Some(from);
            max_from_id = max_from_id.max(from);
            blocks.push(BlockEntry {
                offset: position + start as u64,
                first_from: from,
            });
            sampled_lines += 1;
            sampled_bytes += (line_end + 1) as u64;
        }
        position += block_bytes;
        progress.set(position.min(file_size));
    }

    if blocks.is_empty() {
        bail!(
            "у файлі edges немає жодного рядка виду `from_id<TAB>to_id`:\n  {}",
            source.display()
        );
    }

    let average_line = (sampled_bytes / sampled_lines.max(1)).max(4);
    let estimated_edges = file_size / average_line;

    let tmp = out.with_extension("part");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_index(
        &tmp,
        &blocks,
        block_bytes,
        file_size,
        sorted,
        estimated_edges,
    )?;
    std::fs::rename(&tmp, out).with_context(|| format!("не вдалося зберегти {}", out.display()))?;
    progress.finish_stage();

    Ok(EdgesSummary {
        sorted_by_from: sorted,
        estimated_edges,
        max_from_id,
    })
}

/// `from_id \t to_id [\t …]`
#[inline]
pub(crate) fn parse_edge_line(line: &[u8]) -> Option<(u32, u32)> {
    let tab = memchr::memchr(b'\t', line)?;
    let from = parse_u32(&line[..tab])?;
    let rest = &line[tab + 1..];
    let end = rest
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    let to = parse_u32(&rest[..end])?;
    Some((from, to))
}

fn write_index(
    path: &Path,
    blocks: &[BlockEntry],
    block_bytes: u64,
    file_size: u64,
    sorted: bool,
    estimated_edges: u64,
) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("не вдалося створити {}", path.display()))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    out.write_all(MAGIC)?;
    out.write_all(&FORMAT.to_le_bytes())?;
    out.write_all(&(block_bytes as u32).to_le_bytes())?;
    out.write_all(&[u8::from(sorted), 0, 0, 0])?;
    out.write_all(&file_size.to_le_bytes())?;
    out.write_all(&(blocks.len() as u64).to_le_bytes())?;
    out.write_all(&estimated_edges.to_le_bytes())?;
    out.write_all(&[0u8; 8])?;
    for block in blocks {
        out.write_all(&block.offset.to_le_bytes())?;
        out.write_all(&block.first_from.to_le_bytes())?;
        out.write_all(&[0u8; 4])?;
    }
    out.flush()?;
    Ok(())
}

impl EdgeIndex {
    pub fn open(index_path: &Path, source: &Path) -> Result<Self> {
        let raw = std::fs::read(index_path)
            .with_context(|| format!("не вдалося прочитати {}", index_path.display()))?;
        if raw.len() < HEADER_BYTES || &raw[..4] != MAGIC {
            bail!("{} не є індексом edges", index_path.display());
        }
        let format = u32::from_le_bytes(raw[4..8].try_into()?);
        if format != FORMAT {
            bail!("індекс edges має несумісну версію {format} (потрібна {FORMAT}) — перебудуйте індекс");
        }
        let sorted_by_from = raw[12] == 1;
        let file_size = u64::from_le_bytes(raw[16..24].try_into()?);
        let block_count = u64::from_le_bytes(raw[24..32].try_into()?) as usize;
        let estimated_edges = u64::from_le_bytes(raw[32..40].try_into()?);

        if raw.len() < HEADER_BYTES + block_count * ENTRY_BYTES {
            bail!("індекс edges обірваний — перебудуйте індекс");
        }
        let mut blocks = Vec::with_capacity(block_count);
        for i in 0..block_count {
            let at = HEADER_BYTES + i * ENTRY_BYTES;
            blocks.push(BlockEntry {
                offset: u64::from_le_bytes(raw[at..at + 8].try_into()?),
                first_from: u32::from_le_bytes(raw[at + 8..at + 12].try_into()?),
            });
        }

        let file = File::open(source)
            .with_context(|| format!("не вдалося відкрити {}", source.display()))?;
        let actual = file.metadata().map(|m| m.len()).unwrap_or(0);
        if actual != file_size {
            bail!(
                "файл edges змінився після побудови індексу ({actual} байт замість {file_size}) — перебудуйте індекс"
            );
        }
        Ok(Self {
            file,
            blocks,
            file_size,
            sorted_by_from,
            estimated_edges,
        })
    }

    /// Every domain id that `from_id` links to, ascending, capped at `limit`.
    ///
    /// Returns `(destinations, truncated)`.
    pub fn outbound(&self, from_id: u32, limit: usize) -> Result<(Vec<u32>, bool)> {
        if !self.sorted_by_from {
            bail!("файл edges не відсортований за from_id — вихідні зв'язки потребують повного сканування");
        }
        let mut out = Vec::new();
        let mut truncated = false;
        if self.blocks.is_empty() {
            return Ok((out, truncated));
        }

        // Start at the last sample whose id is <= from_id; the run may begin
        // anywhere inside that block.
        let position = self.blocks.partition_point(|b| b.first_from <= from_id);
        if position == 0 {
            return Ok((out, truncated));
        }
        let mut cursor = self.blocks[position - 1].offset;
        let mut started = false;

        'outer: while cursor < self.file_size {
            let bytes = read_range(&self.file, cursor, 1 << 20)?;
            if bytes.is_empty() {
                break;
            }
            // Keep only complete lines unless this is the file's tail.
            let usable = match memchr::memrchr(b'\n', &bytes) {
                Some(pos) => pos + 1,
                None => bytes.len(),
            };
            let mut offset = 0usize;
            while offset < usable {
                let end = memchr::memchr(b'\n', &bytes[offset..])
                    .map(|p| offset + p + 1)
                    .unwrap_or(usable);
                let line = trim_eol(&bytes[offset..end]);
                offset = end;
                let Some((from, to)) = parse_edge_line(line) else {
                    continue;
                };
                if from < from_id {
                    continue;
                }
                if from > from_id {
                    break 'outer;
                }
                started = true;
                if out.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                if to != from_id {
                    out.push(to);
                }
            }
            cursor += usable as u64;
            if usable == 0 {
                break;
            }
        }
        let _ = started;
        out.dedup();
        Ok((out, truncated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> std::path::PathBuf {
        // 400 sources × 40 destinations, sorted by from then to — ~9 MB, so the
        // 4 MB sampling produces several blocks.
        let mut text = String::with_capacity(9 << 20);
        for from in 0..400u32 {
            for step in 0..40u32 {
                text.push_str(&format!("{from}\t{}\n", 100_000 + from * 40 + step));
            }
            // Padding comments keep the file big enough to span sample blocks
            // without needing millions of edges in a unit test.
            for _ in 0..300 {
                text.push_str(&format!("{from}\t{}\n", 900_000 + from));
            }
        }
        let path = dir.join("edges.txt");
        std::fs::write(&path, text).expect("write fixture");
        path
    }

    #[test]
    fn finds_the_outbound_run_of_a_domain() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = fixture(dir.path());
        let index_path = dir.path().join("edges.idx");

        let summary = build_with_block_size(
            &source,
            &index_path,
            64 << 10,
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build");
        assert!(summary.sorted_by_from);
        assert!(summary.estimated_edges > 0);

        let index = EdgeIndex::open(&index_path, &source).expect("open");
        assert!(
            index.blocks.len() > 1,
            "fixture must span several sample blocks"
        );

        for probe in [0u32, 1, 137, 399] {
            let (destinations, truncated) = index.outbound(probe, 10_000).expect("outbound");
            assert!(!truncated);
            assert!(
                destinations.contains(&(100_000 + probe * 40)),
                "missing first destination of {probe}"
            );
            assert!(
                destinations.contains(&(100_000 + probe * 40 + 39)),
                "missing last destination of {probe}"
            );
            assert!(
                destinations.iter().all(|&to| to >= 100_000),
                "leaked a neighbour of another domain into {probe}"
            );
        }

        assert!(index.outbound(100_000, 10).expect("absent").0.is_empty());
    }

    #[test]
    fn honours_the_limit_and_reports_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = fixture(dir.path());
        let index_path = dir.path().join("edges.idx");
        build_with_block_size(
            &source,
            &index_path,
            64 << 10,
            &Progress::silent(),
            0.0,
            1.0,
        )
        .expect("build");
        let index = EdgeIndex::open(&index_path, &source).expect("open");

        let (destinations, truncated) = index.outbound(7, 5).expect("outbound");
        assert_eq!(destinations.len(), 5);
        assert!(truncated, "limit reached but truncation not reported");
    }
}
