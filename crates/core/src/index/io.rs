//! Low-level file access shared by the index builders.
//!
//! Two access patterns, both cross-platform:
//! * [`ChunkReader`] — sequential, big buffers, whole-line slices (index builds);
//! * [`read_at`] — positioned reads that need no `&mut File`, so a query can hit
//!   the same handle from several places without locking (block lookups).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

/// Positioned read. Returns how many bytes were actually read.
///
/// `pread` on Unix, `seek_read` on Windows — neither moves a shared cursor,
/// which is what lets [`crate::index::vertices::VertexIndex`] stay `Sync`.
pub fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        file.seek_read(buf, offset)
    }
    #[cfg(not(any(unix, windows)))]
    {
        use std::io::{Seek, SeekFrom};
        let mut handle = file.try_clone()?;
        handle.seek(SeekFrom::Start(offset))?;
        handle.read(buf)
    }
}

/// Read exactly `len` bytes at `offset`, or fewer at end of file.
pub fn read_range(file: &File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        let n = read_at(file, offset + filled as u64, &mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// Sequential reader that hands out slices ending on a line boundary.
///
/// A partial trailing line is carried over into the next chunk, so callers
/// never see a truncated record — the single most common source of silently
/// wrong counts when parsing multi-GB text.
pub struct ChunkReader {
    inner: Box<dyn Read + Send>,
    buf: Vec<u8>,
    /// Valid bytes currently in `buf`.
    len: usize,
    /// Bytes handed to the caller by the previous call, dropped on the next one.
    taken: usize,
    /// File offset of `buf[0]`.
    base: u64,
    chunk: usize,
    eof: bool,
    pub file_size: u64,
}

impl ChunkReader {
    /// Open a plain or gzip-compressed text file.
    ///
    /// For `.gz` the reported `file_size` is the *compressed* size, so progress
    /// derived from it runs ahead — acceptable for the scan fallback, and the
    /// reason the index refuses compressed sources outright.
    pub fn open(path: &Path, chunk: usize) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("не вдалося відкрити {}", path.display()))?;
        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let compressed = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
        let inner: Box<dyn Read + Send> = if compressed {
            // MultiGzDecoder handles concatenated members, as published on S3.
            Box::new(flate2::read::MultiGzDecoder::new(file))
        } else {
            Box::new(file)
        };
        Ok(Self::from_reader(inner, file_size, chunk))
    }

    /// Read an already-open file from its current position.
    pub fn from_file(file: File, chunk: usize) -> Self {
        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Self::from_reader(Box::new(file), file_size, chunk)
    }

    fn from_reader(inner: Box<dyn Read + Send>, file_size: u64, chunk: usize) -> Self {
        Self {
            inner,
            buf: vec![0u8; chunk * 2],
            len: 0,
            taken: 0,
            base: 0,
            chunk,
            eof: false,
            file_size,
        }
    }

    /// Next batch of complete lines together with the file offset it starts at.
    ///
    /// Returns `None` at end of file. The slice stays valid until the next call.
    pub fn next_lines(&mut self) -> Result<Option<(&[u8], u64)>> {
        // Retire the slice returned last time; doing it here (rather than after
        // handing it out) is what keeps the borrow checker happy.
        if self.taken > 0 {
            self.buf.copy_within(self.taken..self.len, 0);
            self.len -= self.taken;
            self.base += self.taken as u64;
            self.taken = 0;
        }

        loop {
            while !self.eof && self.len < self.chunk {
                if self.buf.len() - self.len < self.chunk {
                    self.buf.resize(self.len + self.chunk, 0);
                }
                let read = self.inner.read(&mut self.buf[self.len..])?;
                if read == 0 {
                    self.eof = true;
                } else {
                    self.len += read;
                }
            }
            if self.len == 0 {
                return Ok(None);
            }

            let end = match memchr::memrchr(b'\n', &self.buf[..self.len]) {
                Some(pos) => pos + 1,
                // Last line of a file with no trailing newline.
                None if self.eof => self.len,
                None => {
                    // A single line longer than the chunk: grow and retry.
                    self.chunk += self.buf.len();
                    self.buf.resize(self.chunk * 2, 0);
                    continue;
                }
            };

            self.taken = end;
            return Ok(Some((&self.buf[..end], self.base)));
        }
    }
}

/// Split a byte buffer of complete lines into `parts` slices, each ending on a
/// line boundary. Used to hand work to rayon without splitting a record.
pub fn split_on_lines(data: &[u8], parts: usize) -> Vec<&[u8]> {
    if parts <= 1 || data.len() < parts {
        return vec![data];
    }
    let mut out = Vec::with_capacity(parts);
    let step = data.len() / parts;
    let mut start = 0;
    for i in 1..parts {
        let probe = (step * i).max(start);
        if probe >= data.len() {
            break;
        }
        match memchr::memchr(b'\n', &data[probe..]) {
            Some(pos) => {
                let end = probe + pos + 1;
                if end > start {
                    out.push(&data[start..end]);
                    start = end;
                }
            }
            None => break,
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}

/// Parse a decimal `u32` from ASCII bytes without an intermediate `String`.
#[inline]
pub fn parse_u32(bytes: &[u8]) -> Option<u32> {
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

/// Parse an `f64` from ASCII bytes (`4.115E-9` and friends).
#[inline]
pub fn parse_f64(bytes: &[u8]) -> Option<f64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Strip a trailing `\n` and `\r` — the same line on Windows and Unix.
#[inline]
pub fn trim_eol(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

/// Fields of a tab-separated line, zero-copy.
#[inline]
pub fn tab_fields(line: &[u8], out: &mut Vec<usize>) {
    out.clear();
    let mut offset = 0;
    while let Some(pos) = memchr::memchr(b'\t', &line[offset..]) {
        out.push(offset + pos);
        offset += pos + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_every_byte_and_never_cuts_a_line() {
        let data = b"aaa\nbb\ncccc\nd\nee\n";
        for parts in 1..6 {
            let chunks = split_on_lines(data, parts);
            assert_eq!(
                chunks.iter().map(|c| c.len()).sum::<usize>(),
                data.len(),
                "parts={parts} lost bytes"
            );
            for chunk in &chunks {
                assert_eq!(*chunk.last().unwrap(), b'\n', "parts={parts} cut a line");
            }
        }
    }

    #[test]
    fn chunk_reader_reconstructs_the_file_exactly() {
        // Lines of uneven length, and a final line without a newline.
        let mut source = String::new();
        for i in 0..5000 {
            source.push_str(&format!("{i}\tcom.example{}\t1\n", "x".repeat(i % 40)));
        }
        source.push_str("9999\tcom.no-trailing-newline\t1");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vertices.txt");
        std::fs::write(&path, &source).expect("write fixture");

        // Small chunks so the leftover path is exercised many times.
        let mut reader = ChunkReader::open(&path, 64).expect("open");
        let mut rebuilt = Vec::new();
        let mut offset = 0u64;
        let mut batches = 0;
        while let Some((slice, base)) = reader.next_lines().expect("read") {
            assert_eq!(base, offset, "offsets must be contiguous");
            offset += slice.len() as u64;
            rebuilt.extend_from_slice(slice);
            batches += 1;
        }
        assert!(batches > 10, "chunking never kicked in ({batches} batches)");
        assert!(!rebuilt.is_empty(), "reader produced nothing");
        assert_eq!(String::from_utf8(rebuilt).unwrap(), source);
    }

    #[test]
    fn parses_numbers_and_trims_both_line_endings() {
        assert_eq!(parse_u32(b"12345"), Some(12345));
        assert_eq!(parse_u32(b"12x"), None);
        assert_eq!(parse_u32(b""), None);
        assert_eq!(
            parse_f64(b"4.115653851122188E-9"),
            Some(4.115653851122188E-9)
        );
        assert_eq!(trim_eol(b"a\tb\r\n"), b"a\tb");
        assert_eq!(trim_eol(b"a\tb\n"), b"a\tb");
        assert_eq!(trim_eol(b"a\tb"), b"a\tb");
    }
}
