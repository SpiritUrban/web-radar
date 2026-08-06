//! Where the graph files come from, and what the user has to do to get them.
//!
//! Web Radar deliberately does **not** download the graph itself: it is ~17 GB
//! compressed from a third party, and a silent multi-hour download inside an
//! app is worse than an honest instruction. So the instruction has to be good.
//!
//! Everything a user needs — exact file names, direct links, both sizes, and
//! which files must be unpacked — lives here, in one place, and is rendered by
//! both the CLI and the desktop app.

use serde::{Deserialize, Serialize};

/// The release used to size and name things when nothing is configured yet.
///
/// Common Crawl publishes a new domain graph roughly every two to three
/// months; the id is the only part of the URL that changes.
pub const DEFAULT_CRAWL: &str = "cc-main-2026-apr-may-jun";

/// Listing of every published graph, for when a newer crawl is out.
pub const CRAWL_LIST_URL: &str = "https://commoncrawl.org/web-graphs";

const BASE: &str = "https://data.commoncrawl.org/projects/hyperlinkgraph";

/// Which of the three files this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphFile {
    Vertices,
    Edges,
    Ranks,
}

impl GraphFile {
    pub const ALL: [GraphFile; 3] = [GraphFile::Vertices, GraphFile::Edges, GraphFile::Ranks];

    pub fn key(self) -> &'static str {
        match self {
            GraphFile::Vertices => "vertices",
            GraphFile::Edges => "edges",
            GraphFile::Ranks => "ranks",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        GraphFile::ALL.into_iter().find(|file| file.key() == key)
    }

    pub fn purpose(self) -> &'static str {
        match self {
            GraphFile::Vertices => "Список доменів: номер і назва кожного з ~121 млн доменів.",
            GraphFile::Edges => "Усі посилання між доменами. Найбільший файл.",
            GraphFile::Ranks => "PageRank і Harmonic для кожного домену.",
        }
    }

    /// Whether queries need random access into this file after indexing.
    ///
    /// `ranks` is read once, sequentially, and never again — so it may stay
    /// gzipped, saving 8 GB of disk and one unpacking step. `vertices` and
    /// `edges` are seeked into on every query, which gzip cannot do.
    pub fn must_be_unpacked(self) -> bool {
        match self {
            GraphFile::Vertices | GraphFile::Edges => true,
            GraphFile::Ranks => false,
        }
    }

    /// Approximate download size for the reference crawl, in bytes.
    pub fn download_bytes(self) -> u64 {
        match self {
            GraphFile::Vertices => 838 * 1024 * 1024,
            GraphFile::Edges => 13_600 * 1024 * 1024,
            GraphFile::Ranks => 2_252 * 1024 * 1024,
        }
    }

    /// Approximate size once unpacked, in bytes.
    pub fn unpacked_bytes(self) -> u64 {
        match self {
            GraphFile::Vertices => 3_430_160_421,
            GraphFile::Edges => 67_065_528_122,
            GraphFile::Ranks => 8_331_642_513,
        }
    }

    /// Disk actually needed for this file after setup: `ranks` may stay packed.
    pub fn resident_bytes(self) -> u64 {
        if self.must_be_unpacked() {
            self.unpacked_bytes()
        } else {
            self.download_bytes()
        }
    }

    pub fn file_name(self, crawl: &str) -> String {
        format!("{crawl}-domain-{}.txt", self.key())
    }

    pub fn archive_name(self, crawl: &str) -> String {
        format!("{}.gz", self.file_name(crawl))
    }

    pub fn download_url(self, crawl: &str) -> String {
        format!("{BASE}/{crawl}/domain/{}", self.archive_name(crawl))
    }
}

/// One row of the "how to get the data" table, ready for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadHint {
    pub kind: String,
    pub purpose: String,
    pub file_name: String,
    pub archive_name: String,
    pub url: String,
    pub download_bytes: u64,
    pub unpacked_bytes: u64,
    pub must_be_unpacked: bool,
}

/// The three rows, plus the totals, for a given crawl.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupGuide {
    pub crawl: String,
    pub crawl_list_url: String,
    pub files: Vec<DownloadHint>,
    pub total_download_bytes: u64,
    pub total_resident_bytes: u64,
}

/// Guess the crawl id from a configured path, so the links point at the release
/// the user already decided on rather than at our default.
pub fn crawl_from_path(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let cut = name.find("-domain-")?;
    let crawl = &name[..cut];
    crawl.starts_with("cc-").then(|| crawl.to_string())
}

pub fn setup_guide(crawl: &str) -> SetupGuide {
    let files: Vec<DownloadHint> = GraphFile::ALL
        .into_iter()
        .map(|file| DownloadHint {
            kind: file.key().to_string(),
            purpose: file.purpose().to_string(),
            file_name: file.file_name(crawl),
            archive_name: file.archive_name(crawl),
            url: file.download_url(crawl),
            download_bytes: file.download_bytes(),
            unpacked_bytes: file.unpacked_bytes(),
            must_be_unpacked: file.must_be_unpacked(),
        })
        .collect();
    SetupGuide {
        crawl: crawl.to_string(),
        crawl_list_url: CRAWL_LIST_URL.to_string(),
        total_download_bytes: GraphFile::ALL.iter().map(|f| f.download_bytes()).sum(),
        total_resident_bytes: GraphFile::ALL.iter().map(|f| f.resident_bytes()).sum(),
        files,
    }
}

/// The whole instruction as plain text, for the CLI and for error messages.
pub fn instructions(crawl: &str, target_dir: &std::path::Path) -> String {
    let guide = setup_guide(crawl);
    let mut out = format!(
        "Як отримати дані ({}):\n\n1. Завантажте три файли (разом ~{}):\n",
        crawl,
        crate::index::human_bytes(guide.total_download_bytes)
    );
    for file in &guide.files {
        out.push_str(&format!(
            "     {}\n       {} — {}\n",
            file.url,
            crate::index::human_bytes(file.download_bytes),
            file.purpose
        ));
    }
    out.push_str(&format!(
        "\n2. Розпакуйте vertices і edges (ranks можна лишити .gz — він читається\n   \
         послідовно і не потребує розпакування).\n\n\
         3. Покладіть файли сюди:\n     {}\n\n\
         4. Запустіть `web-radar index build`.\n\n\
         Разом на диску після налаштування: ~{}. Новий випуск графа виходить\n\
         раз на 2–3 місяці — перелік: {}\n",
        target_dir.display(),
        crate::index::human_bytes(guide.total_resident_bytes),
        CRAWL_LIST_URL,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn builds_the_urls_common_crawl_actually_serves() {
        // Verified against data.commoncrawl.org: HTTP 200 with Accept-Ranges.
        assert_eq!(
            GraphFile::Edges.download_url("cc-main-2026-apr-may-jun"),
            "https://data.commoncrawl.org/projects/hyperlinkgraph/cc-main-2026-apr-may-jun/domain/cc-main-2026-apr-may-jun-domain-edges.txt.gz"
        );
        assert_eq!(
            GraphFile::Vertices.file_name("cc-main-2026-apr-may-jun"),
            "cc-main-2026-apr-may-jun-domain-vertices.txt"
        );
    }

    #[test]
    fn only_the_seeked_files_have_to_be_unpacked() {
        assert!(GraphFile::Vertices.must_be_unpacked());
        assert!(GraphFile::Edges.must_be_unpacked());
        // ranks is read once, sequentially — 8 GB of disk saved.
        assert!(!GraphFile::Ranks.must_be_unpacked());
        assert_eq!(
            GraphFile::Ranks.resident_bytes(),
            GraphFile::Ranks.download_bytes()
        );
    }

    #[test]
    fn recognises_the_crawl_from_a_configured_path() {
        let mut path = std::path::PathBuf::from("D:");
        path.push("graphs");
        path.push("cc-main-2025-oct-nov-dec-domain-edges.txt");
        assert_eq!(
            crawl_from_path(&path).as_deref(),
            Some("cc-main-2025-oct-nov-dec")
        );

        assert_eq!(crawl_from_path(Path::new("edges.txt")), None);
        assert_eq!(crawl_from_path(Path::new("my-own-domain-edges.txt")), None);
    }

    #[test]
    fn the_instruction_names_every_file_and_where_to_put_it() {
        let mut dir = std::path::PathBuf::from("D:");
        dir.push("graphs");
        let text = instructions(DEFAULT_CRAWL, &dir);

        for file in GraphFile::ALL {
            assert!(
                text.contains(&file.download_url(DEFAULT_CRAWL)),
                "missing the link for {}",
                file.key()
            );
        }
        assert!(
            text.contains("Розпакуйте"),
            "must say the archives need unpacking"
        );
        assert!(
            text.contains(&dir.display().to_string()),
            "must say where to put them"
        );
        assert!(
            text.contains(CRAWL_LIST_URL),
            "must say where newer crawls are listed"
        );
    }
}
