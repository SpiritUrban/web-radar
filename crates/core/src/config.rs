//! Configuration loading from `config.toml`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::index::GraphSources;
use crate::reverse::{normalize_domain, to_reverse};

/// Which centrality metric from the ranks file to export as `rank`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RankMetric {
    #[default]
    Pagerank,
    Harmonic,
}

impl RankMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            RankMetric::Pagerank => "pagerank",
            RankMetric::Harmonic => "harmonic",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pagerank" | "pr" => Ok(RankMetric::Pagerank),
            "harmonic" | "harmonicc" => Ok(RankMetric::Harmonic),
            other => bail!("невідома метрика «{other}» — доступні pagerank і harmonic"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PathsConfig {
    pub vertices: PathBuf,
    pub edges: PathBuf,
    pub ranks: PathBuf,
    #[serde(default = "default_results_dir")]
    pub results_dir: PathBuf,
    /// Where the index lives. Defaults to `index/` next to the config file —
    /// it can be gigabytes, so it is worth pointing at a roomy drive.
    #[serde(default = "default_index_dir")]
    pub index_dir: PathBuf,
}

fn default_results_dir() -> PathBuf {
    PathBuf::from("results")
}

fn default_index_dir() -> PathBuf {
    PathBuf::from("index")
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
    pub domain: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub paths: PathsConfig,
    #[serde(default)]
    pub rank_metric: RankMetric,
    #[serde(default)]
    pub targets: Vec<Target>,
    /// Directory that contained the config file (used to resolve relative paths).
    #[serde(skip)]
    pub config_dir: PathBuf,
}

/// Resolved target ready for processing.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    /// Clean hostname (`example.com`).
    pub domain: String,
    /// Reverse domain notation (`com.example`).
    pub reverse: String,
}

impl Config {
    /// Load config from a TOML file.
    ///
    /// Relative paths inside the config are resolved against the config file's
    /// directory, so running the binary from any cwd still works.
    ///
    /// Missing graph files are **not** an error here: the desktop app has to be
    /// able to open, show what is missing and let the user fix it. Call
    /// [`Config::ensure_files`] before starting work that needs them.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };

        let raw = fs::read_to_string(&path).with_context(|| {
            format!("не вдалося прочитати файл конфігурації {}", path.display())
        })?;
        let mut cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("не вдалося розібрати {}", path.display()))?;

        let config_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        cfg.resolve_relative_to(&config_dir);
        Ok(cfg)
    }

    /// Resolve every relative path against `config_dir` and unwrap
    /// single-file directories (the shape you get from extracting an archive).
    pub fn resolve_relative_to(&mut self, config_dir: &Path) {
        self.config_dir = config_dir.to_path_buf();
        self.paths.vertices = resolve_path(config_dir, &self.paths.vertices);
        self.paths.edges = resolve_path(config_dir, &self.paths.edges);
        self.paths.ranks = resolve_path(config_dir, &self.paths.ranks);
        self.paths.results_dir = resolve_path(config_dir, &self.paths.results_dir);
        self.paths.index_dir = resolve_path(config_dir, &self.paths.index_dir);

        self.paths.vertices = unwrap_single_file_dir(&self.paths.vertices, "vertices");
        self.paths.edges = unwrap_single_file_dir(&self.paths.edges, "edges");
        self.paths.ranks = unwrap_single_file_dir(&self.paths.ranks, "ranks");
    }

    /// The three source files, for the index and query layers.
    pub fn sources(&self) -> GraphSources {
        GraphSources {
            vertices: self.paths.vertices.clone(),
            edges: self.paths.edges.clone(),
            ranks: self.paths.ranks.clone(),
        }
    }

    /// Fail early, and with an actionable message, when a graph file is absent.
    pub fn ensure_files(&self) -> Result<()> {
        for (label, path) in [
            ("vertices", &self.paths.vertices),
            ("edges", &self.paths.edges),
            ("ranks", &self.paths.ranks),
        ] {
            if !path.exists() {
                bail!(
                    "не знайдено файл {label}:\n  {}\n\n\
                     Покладіть файли графа доменів Common Crawl поруч із config.toml або виправте шлях у [paths].\n\
                     Потрібні три файли: *-domain-vertices.txt, *-domain-edges.txt, *-domain-ranks.txt\n\
                     (завантажити: {})\n\n\
                     Швидка перевірка без багатогігабайтних завантажень:\n  web-radar run -c testdata/config.toml",
                    path.display(),
                    crate::meta::DATA_SOURCE_URL,
                );
            }
            if path.is_dir() {
                bail!(
                    "шлях {label} веде до теки, а не до файла:\n  {}\n\
                     Вкажіть .txt / .txt.gz усередині неї.",
                    path.display()
                );
            }
        }
        Ok(())
    }

    /// Resolve config targets to reverse-domain form (deduplicated).
    pub fn resolved_targets(&self) -> Result<Vec<ResolvedTarget>> {
        let mut out = Vec::with_capacity(self.targets.len());
        let mut seen = ahash::AHashSet::new();

        for target in &self.targets {
            let domain = normalize_domain(&target.domain);
            let reverse = to_reverse(&domain);
            if reverse.is_empty() {
                bail!("некоректний домен: «{}»", target.domain);
            }
            if !seen.insert(reverse.clone()) {
                log::warn!("duplicate target domain skipped: {domain}");
                continue;
            }
            out.push(ResolvedTarget { domain, reverse });
        }

        if out.is_empty() {
            bail!("у конфігурації немає жодного домену — додайте [[targets]]");
        }
        Ok(out)
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// If `path` is a directory with exactly one file whose name contains `hint`,
/// return that file. Helps when someone extracts an archive into a folder.
fn unwrap_single_file_dir(path: &Path, hint: &str) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return path.to_path_buf();
    };
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.is_file()
                && candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().contains(hint))
        })
        .collect();
    if matches.len() == 1 {
        log::info!(
            "using {} inside directory {}",
            matches[0].display(),
            path.display()
        );
        return matches.pop().unwrap_or_default();
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [paths]
        vertices = "v.txt"
        edges = "e.txt"
        ranks = "r.txt"

        [[targets]]
        domain = "https://www.Example.COM/path"
    "#;

    #[test]
    fn parses_a_minimal_config_and_normalises_targets() {
        let cfg: Config = toml::from_str(MINIMAL).expect("parse");
        assert_eq!(cfg.rank_metric, RankMetric::Pagerank);
        assert_eq!(cfg.paths.results_dir, PathBuf::from("results"));
        assert_eq!(cfg.paths.index_dir, PathBuf::from("index"));

        let resolved = cfg.resolved_targets().expect("resolve");
        assert_eq!(resolved[0].domain, "example.com");
        assert_eq!(resolved[0].reverse, "com.example");
    }

    #[test]
    fn relative_paths_resolve_against_the_config_directory() {
        let mut cfg: Config = toml::from_str(MINIMAL).expect("parse");
        // Built with `push`, never with a hardcoded separator: the same test
        // has to mean the same thing on Windows and on the Linux CI runner.
        let mut base = PathBuf::from("data");
        base.push("graphs");
        cfg.resolve_relative_to(&base);

        let mut expected = base.clone();
        expected.push("v.txt");
        assert_eq!(cfg.paths.vertices, expected);

        let mut expected_index = base;
        expected_index.push("index");
        assert_eq!(cfg.paths.index_dir, expected_index);
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        let mut cfg: Config = toml::from_str(MINIMAL).expect("parse");
        let absolute = std::env::current_dir().expect("cwd").join("elsewhere.txt");
        cfg.paths.edges = absolute.clone();
        cfg.resolve_relative_to(Path::new("data"));
        assert_eq!(cfg.paths.edges, absolute);
    }

    #[test]
    fn missing_files_produce_a_message_that_says_what_to_do() {
        let cfg: Config = toml::from_str(MINIMAL).expect("parse");
        let error = cfg.ensure_files().expect_err("files do not exist");
        let text = format!("{error}");
        assert!(text.contains("vertices"), "must name the file: {text}");
        assert!(
            text.contains("commoncrawl.org"),
            "must say where to get it: {text}"
        );
    }

    #[test]
    fn rank_metric_round_trips_through_its_string_form() {
        for metric in [RankMetric::Pagerank, RankMetric::Harmonic] {
            assert_eq!(RankMetric::parse(metric.as_str()).expect("parse"), metric);
        }
        assert!(RankMetric::parse("betweenness").is_err());
    }
}
