//! Sparse block index over `*-domain-vertices.txt`.
//!
//! The file is `id \t reverse_domain \t n_hosts`, sorted by `reverse_domain`,
//! with `id` equal to the line number. Both orderings are therefore usable for
//! binary search — we only need to know where the lines are.
//!
//! The index keeps one entry per [`BLOCK_LINES`] lines: the byte offset, the id
//! and the domain of the block's first line. For the 2026 apr–jun graph that is
//! ~473 000 entries (~19 MB) covering 121 million domains, so a lookup is one
//! binary search in RAM plus a single ~7 KB read.
//!
//! The build **verifies** both orderings instead of assuming them; a file that
//! turns out unsorted disables the affected lookup rather than answering wrongly.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use ahash::{AHashMap, AHashSet};
use anyhow::{bail, Context, Result};

use super::io::{parse_u32, read_range, trim_eol, ChunkReader};
use crate::progress::Progress;

const MAGIC: &[u8; 4] = b"WRVI";
const FORMAT: u32 = 1;
/// Lines per block: 256 keeps a block at ~7 KB, one page cluster per lookup.
pub const BLOCK_LINES: u32 = 256;
const HEADER_BYTES: usize = 48;
const ENTRY_BYTES: usize = 20;

struct BlockEntry {
    offset: u64,
    first_id: u32,
    name_start: u32,
    name_len: u16,
}

/// Loaded vertex block index plus an open handle on the source file.
pub struct VertexIndex {
    file: File,
    source: std::path::PathBuf,
    blocks: Vec<BlockEntry>,
    names: Vec<u8>,
    pub node_count: u64,
    pub file_size: u64,
    pub sorted_by_name: bool,
    pub sorted_by_id: bool,
}

/// What the single vertices pass learned about the file.
#[derive(Debug, Clone, Copy)]
pub struct VerticesSummary {
    pub node_count: u64,
    pub sorted_by_name: bool,
    pub sorted_by_id: bool,
    pub max_id: u32,
}

/// Build the block index with one sequential pass over the vertices file.
///
/// `base`/`weight` place this pass inside the caller's overall progress bar.
pub fn build(
    source: &Path,
    out: &Path,
    progress: &Progress,
    base: f64,
    weight: f64,
) -> Result<VerticesSummary> {
    let mut reader = ChunkReader::open(source, 8 << 20)?;
    let total = reader.file_size;
    progress.stage(
        "vertices_index",
        format!("Індексуємо домени · {}", file_label(source)),
        base,
        weight,
        total,
    );

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("part");
    let mut blocks: Vec<BlockEntry> = Vec::new();
    let mut names: Vec<u8> = Vec::new();

    let mut line_no: u64 = 0;
    let mut node_count: u64 = 0;
    let mut max_id: u32 = 0;
    let mut sorted_by_name = true;
    let mut sorted_by_id = true;
    let mut previous_name: Vec<u8> = Vec::new();
    let mut previous_id: Option<u32> = None;

    while let Some((chunk, chunk_base)) = reader.next_lines()? {
        progress.check()?;
        let mut cursor = 0usize;
        while cursor < chunk.len() {
            let end = memchr::memchr(b'\n', &chunk[cursor..])
                .map(|p| cursor + p + 1)
                .unwrap_or(chunk.len());
            let line_offset = chunk_base + cursor as u64;
            let line = trim_eol(&chunk[cursor..end]);
            cursor = end;
            if line.is_empty() || line[0] == b'#' {
                continue;
            }
            let Some((id, name)) = parse_vertex_line(line) else {
                continue;
            };

            if !previous_name.is_empty() && name <= previous_name.as_slice() {
                sorted_by_name = false;
            }
            previous_name.clear();
            previous_name.extend_from_slice(name);
            if previous_id.is_some_and(|p| id <= p) {
                sorted_by_id = false;
            }
            previous_id = Some(id);

            if line_no.is_multiple_of(u64::from(BLOCK_LINES)) {
                blocks.push(BlockEntry {
                    offset: line_offset,
                    first_id: id,
                    name_start: names.len() as u32,
                    name_len: name.len().min(u16::MAX as usize) as u16,
                });
                names.extend_from_slice(&name[..name.len().min(u16::MAX as usize)]);
            }
            line_no += 1;
            node_count += 1;
            max_id = max_id.max(id);
        }
        progress.set(chunk_base + chunk.len() as u64);
    }

    if node_count == 0 {
        bail!(
            "у файлі vertices немає жодного рядка виду `id<TAB>домен`:\n  {}",
            source.display()
        );
    }

    let summary = VerticesSummary {
        node_count,
        sorted_by_name,
        sorted_by_id,
        max_id,
    };
    write_index(&tmp, &blocks, &names, reader.file_size, &summary)?;
    std::fs::rename(&tmp, out).with_context(|| format!("не вдалося зберегти {}", out.display()))?;
    progress.finish_stage();
    Ok(summary)
}

/// `id \t reverse_domain [\t n_hosts]`
#[inline]
fn parse_vertex_line(line: &[u8]) -> Option<(u32, &[u8])> {
    let tab = memchr::memchr(b'\t', line)?;
    let id = parse_u32(&line[..tab])?;
    let rest = &line[tab + 1..];
    let name_end = memchr::memchr(b'\t', rest).unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        None
    } else {
        Some((id, name))
    }
}

fn write_index(
    path: &Path,
    blocks: &[BlockEntry],
    names: &[u8],
    file_size: u64,
    summary: &VerticesSummary,
) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("не вдалося створити {}", path.display()))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    out.write_all(MAGIC)?; // 0
    out.write_all(&FORMAT.to_le_bytes())?; // 4
    out.write_all(&BLOCK_LINES.to_le_bytes())?; // 8
    out.write_all(&(names.len() as u32).to_le_bytes())?; // 12
    out.write_all(&summary.node_count.to_le_bytes())?; // 16
    out.write_all(&file_size.to_le_bytes())?; // 24
    out.write_all(&(blocks.len() as u64).to_le_bytes())?; // 32
    out.write_all(&[
        u8::from(summary.sorted_by_name),
        u8::from(summary.sorted_by_id),
        0,
        0,
        0,
        0,
        0,
        0,
    ])?; // 40..48

    for block in blocks {
        out.write_all(&block.offset.to_le_bytes())?; // +0
        out.write_all(&block.first_id.to_le_bytes())?; // +8
        out.write_all(&block.name_start.to_le_bytes())?; // +12
        out.write_all(&block.name_len.to_le_bytes())?; // +16
        out.write_all(&[0, 0])?; // +18, entry = 20 bytes
    }
    out.write_all(names)?;
    out.flush()?;
    Ok(())
}

impl VertexIndex {
    /// Load the index into RAM and open the source file for block reads.
    pub fn open(index_path: &Path, source: &Path) -> Result<Self> {
        let raw = std::fs::read(index_path)
            .with_context(|| format!("не вдалося прочитати {}", index_path.display()))?;
        if raw.len() < HEADER_BYTES || &raw[..4] != MAGIC {
            bail!("{} не є індексом vertices", index_path.display());
        }
        let format = u32::from_le_bytes(raw[4..8].try_into()?);
        if format != FORMAT {
            bail!("індекс vertices має несумісну версію {format} (потрібна {FORMAT}) — перебудуйте індекс");
        }
        let names_len = u32::from_le_bytes(raw[12..16].try_into()?) as usize;
        let node_count = u64::from_le_bytes(raw[16..24].try_into()?);
        let file_size = u64::from_le_bytes(raw[24..32].try_into()?);
        let block_count = u64::from_le_bytes(raw[32..40].try_into()?) as usize;
        let sorted_by_name = raw[40] == 1;
        let sorted_by_id = raw[41] == 1;

        let entries_end = HEADER_BYTES + block_count * ENTRY_BYTES;
        if raw.len() < entries_end + names_len {
            bail!("індекс vertices обірваний — перебудуйте індекс");
        }
        let mut blocks = Vec::with_capacity(block_count);
        for i in 0..block_count {
            let at = HEADER_BYTES + i * ENTRY_BYTES;
            blocks.push(BlockEntry {
                offset: u64::from_le_bytes(raw[at..at + 8].try_into()?),
                first_id: u32::from_le_bytes(raw[at + 8..at + 12].try_into()?),
                name_start: u32::from_le_bytes(raw[at + 12..at + 16].try_into()?),
                name_len: u16::from_le_bytes(raw[at + 16..at + 18].try_into()?),
            });
        }
        let names = raw[entries_end..entries_end + names_len].to_vec();

        let file = File::open(source)
            .with_context(|| format!("не вдалося відкрити {}", source.display()))?;
        let actual = file.metadata().map(|m| m.len()).unwrap_or(0);
        if actual != file_size {
            bail!(
                "файл vertices змінився після побудови індексу ({actual} байт замість {file_size}) — перебудуйте індекс"
            );
        }

        Ok(Self {
            file,
            source: source.to_path_buf(),
            blocks,
            names,
            node_count,
            file_size,
            sorted_by_name,
            sorted_by_id,
        })
    }

    fn block_name(&self, i: usize) -> &[u8] {
        let block = &self.blocks[i];
        let start = block.name_start as usize;
        &self.names[start..start + block.name_len as usize]
    }

    fn block_bytes(&self, i: usize) -> Result<Vec<u8>> {
        let start = self.blocks[i].offset;
        let end = self.block_end(i);
        Ok(read_range(&self.file, start, (end - start) as usize)?)
    }

    /// Node id of a reverse domain (`com.example`), if the graph knows it.
    pub fn id_of(&self, reverse_domain: &str) -> Result<Option<u32>> {
        if !self.sorted_by_name {
            bail!("файл vertices не відсортований за доменом — пошук за назвою потребує повного сканування");
        }
        if self.blocks.is_empty() {
            return Ok(None);
        }
        let needle = reverse_domain.as_bytes();
        // Last block whose first name is <= needle.
        let mut lo = 0usize;
        let mut hi = self.blocks.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.block_name(mid) <= needle {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return Ok(None);
        }
        let bytes = self.block_bytes(lo - 1)?;
        let mut found = None;
        for_each_vertex(&bytes, |id, name| {
            if name.as_bytes() == needle {
                found = Some(id);
            }
        });
        Ok(found)
    }

    /// Reverse domain of a single node id.
    pub fn name_of(&self, id: u32) -> Result<Option<String>> {
        let mut found = None;
        self.names_of(&[id], |node, name| {
            if node == id {
                found = Some(name.to_string());
            }
        })?;
        Ok(found)
    }

    /// Resolve many ids at once; `visit` is called per resolved id, unordered.
    ///
    /// Scattered ids cost one ~7 KB block read each. Once a request touches a
    /// large share of the blocks, one sequential pass is cheaper — so it switches.
    pub fn names_of(&self, ids: &[u32], mut visit: impl FnMut(u32, &str)) -> Result<usize> {
        if ids.is_empty() || self.blocks.is_empty() {
            return Ok(0);
        }
        if !self.sorted_by_id {
            bail!("ідентифікатори у файлі vertices не зростають — індекс за id недоступний");
        }
        let mut wanted: Vec<u32> = ids.to_vec();
        wanted.sort_unstable();
        wanted.dedup();

        let mut needed: Vec<usize> = wanted.iter().map(|&id| self.block_of_id(id)).collect();
        needed.dedup();

        let mut resolved = 0usize;
        if needed.len() * 4 > self.blocks.len() {
            // Dense request: read the file once instead of a quarter of it in
            // random 7 KB pieces.
            let lookup: AHashSet<u32> = wanted.iter().copied().collect();
            let mut reader = ChunkReader::open(&self.source, 8 << 20)?;
            while let Some((chunk, _)) = reader.next_lines()? {
                for_each_vertex(chunk, |id, name| {
                    if lookup.contains(&id) {
                        resolved += 1;
                        visit(id, name);
                    }
                });
            }
            return Ok(resolved);
        }

        for block in needed {
            let bytes = self.block_bytes(block)?;
            let first = self.blocks[block].first_id;
            let past = self
                .blocks
                .get(block + 1)
                .map(|b| b.first_id)
                .unwrap_or(u32::MAX);
            let from = wanted.partition_point(|&id| id < first);
            let to = wanted.partition_point(|&id| id < past);
            let want_here: AHashSet<u32> = wanted[from..to].iter().copied().collect();
            if want_here.is_empty() {
                continue;
            }
            for_each_vertex(&bytes, |id, name| {
                if want_here.contains(&id) {
                    resolved += 1;
                    visit(id, name);
                }
            });
        }
        Ok(resolved)
    }

    /// Index of the block that would contain `reverse_domain`.
    ///
    /// Used by the ranks join to bucket rows by their position in the file
    /// without resolving each name to an id first.
    pub fn block_of_name(&self, reverse_domain: &[u8]) -> usize {
        let mut lo = 0usize;
        let mut hi = self.blocks.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.block_name(mid) <= reverse_domain {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1)
    }

    fn block_of_id(&self, id: u32) -> usize {
        self.blocks
            .partition_point(|b| b.first_id <= id)
            .saturating_sub(1)
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// First id stored in block `i`.
    pub fn block_first_id(&self, i: usize) -> u32 {
        self.blocks[i].first_id
    }

    /// Byte offset just past block `i`.
    fn block_end(&self, i: usize) -> u64 {
        self.blocks
            .get(i + 1)
            .map(|b| b.offset)
            .unwrap_or(self.file_size)
    }

    /// `reverse_domain -> id` for blocks `[from, to)`, used by the ranks join.
    pub fn names_in_blocks(&self, from: usize, to: usize) -> Result<AHashMap<String, u32>> {
        if from >= to || to > self.blocks.len() {
            return Ok(AHashMap::new());
        }
        let start = self.blocks[from].offset;
        let end = self.block_end(to - 1);
        let bytes = read_range(&self.file, start, (end - start) as usize)?;
        let mut map = AHashMap::with_capacity((to - from) * BLOCK_LINES as usize);
        for_each_vertex(&bytes, |id, name| {
            map.insert(name.to_string(), id);
        });
        Ok(map)
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
        if let Some((id, name)) = parse_vertex_line(line) {
            if let Ok(text) = std::str::from_utf8(name) {
                visit(id, text);
            }
        }
    }
}

pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> std::path::PathBuf {
        // 2000 domains in reverse-domain order, ids equal to line numbers.
        let mut text = String::new();
        for i in 0..2000u32 {
            text.push_str(&format!("{i}\tcom.site{i:05}\t1\n"));
        }
        let path = dir.join("vertices.txt");
        std::fs::write(&path, text).expect("write fixture");
        path
    }

    #[test]
    fn resolves_names_and_ids_in_both_directions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = fixture(dir.path());
        let index_path = dir.path().join("vertices.idx");

        let summary = build(&source, &index_path, &Progress::silent(), 0.0, 1.0).expect("build");
        assert_eq!(summary.node_count, 2000);
        assert!(summary.sorted_by_name && summary.sorted_by_id);

        let index = VertexIndex::open(&index_path, &source).expect("open");
        assert!(index.block_count() > 1, "fixture must span several blocks");

        assert_eq!(index.id_of("com.site00000").expect("lookup"), Some(0));
        assert_eq!(index.id_of("com.site01999").expect("lookup"), Some(1999));
        assert_eq!(index.id_of("com.site00777").expect("lookup"), Some(777));
        assert_eq!(index.id_of("com.nothing-here").expect("lookup"), None);
        assert_eq!(
            index.name_of(1234).expect("name"),
            Some("com.site01234".into())
        );

        let mut seen = Vec::new();
        let resolved = index
            .names_of(&[5, 300, 301, 1998], |id, name| {
                seen.push((id, name.to_string()))
            })
            .expect("batch");
        assert_eq!(resolved, 4, "batch resolution lost ids: {seen:?}");
        seen.sort();
        assert_eq!(seen[0], (5, "com.site00005".to_string()));
        assert_eq!(seen[3], (1998, "com.site01998".to_string()));
    }

    #[test]
    fn dense_batch_takes_the_sequential_path_and_still_resolves_everything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = fixture(dir.path());
        let index_path = dir.path().join("vertices.idx");
        build(&source, &index_path, &Progress::silent(), 0.0, 1.0).expect("build");
        let index = VertexIndex::open(&index_path, &source).expect("open");

        let ids: Vec<u32> = (0..2000).step_by(3).collect();
        let mut seen = 0usize;
        let resolved = index.names_of(&ids, |_, name| {
            assert!(name.starts_with("com.site"));
            seen += 1;
        });
        assert_eq!(resolved.expect("batch"), ids.len());
        assert_eq!(seen, ids.len());
    }

    #[test]
    fn refuses_name_lookup_when_the_file_is_not_sorted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("unsorted.txt");
        std::fs::write(&path, "0\tcom.b\t1\n1\tcom.a\t1\n2\tcom.c\t1\n").expect("write");
        let index_path = dir.path().join("unsorted.idx");

        let summary = build(&path, &index_path, &Progress::silent(), 0.0, 1.0).expect("build");
        assert!(
            !summary.sorted_by_name,
            "unsorted input must be reported as such"
        );
        assert!(summary.sorted_by_id);

        let index = VertexIndex::open(&index_path, &path).expect("open");
        // Wrong answers are worse than no answers.
        assert!(index.id_of("com.a").is_err());
        // Ids still ascend, so resolving them must keep working.
        assert_eq!(index.name_of(1).expect("name"), Some("com.a".into()));
    }

    #[test]
    fn rejects_an_index_whose_source_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = fixture(dir.path());
        let index_path = dir.path().join("vertices.idx");
        build(&source, &index_path, &Progress::silent(), 0.0, 1.0).expect("build");

        let mut text = std::fs::read_to_string(&source).expect("read");
        text.push_str("2000\tcom.site02000\t1\n");
        std::fs::write(&source, text).expect("append");

        let error = VertexIndex::open(&index_path, &source)
            .err()
            .expect("stale index must fail");
        assert!(
            format!("{error}").contains("перебудуйте"),
            "unhelpful: {error}"
        );
    }
}
