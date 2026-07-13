//! Configuration loading from `config.toml`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::reverse::{normalize_domain, to_reverse};

/// Which centrality metric from the ranks file to export as `rank`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathsConfig {
    pub vertices: PathBuf,
    pub edges: PathBuf,
    pub ranks: PathBuf,
    #[serde(default = "default_results_dir")]
    pub results_dir: PathBuf,
}

fn default_results_dir() -> PathBuf {
    PathBuf::from("results")
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    pub domain: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub paths: PathsConfig,
    #[serde(default)]
    pub rank_metric: RankMetric,
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
    /// Load and validate config from a TOML file.
    ///
    /// Relative paths inside the config are resolved against the config file's
    /// directory (so running the exe from any cwd still works).
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        let config_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        cfg.config_dir = config_dir.clone();

        // Resolve relative paths against the config directory.
        cfg.paths.vertices = resolve_path(&config_dir, &cfg.paths.vertices);
        cfg.paths.edges = resolve_path(&config_dir, &cfg.paths.edges);
        cfg.paths.ranks = resolve_path(&config_dir, &cfg.paths.ranks);
        cfg.paths.results_dir = resolve_path(&config_dir, &cfg.paths.results_dir);

        // If a path points at a directory that contains a single matching file, enter it.
        cfg.paths.vertices = unwrap_single_file_dir(&cfg.paths.vertices, "vertices");
        cfg.paths.edges = unwrap_single_file_dir(&cfg.paths.edges, "edges");
        cfg.paths.ranks = unwrap_single_file_dir(&cfg.paths.ranks, "ranks");

        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.targets.is_empty() {
            bail!("config has no [[targets]]; add at least one domain");
        }
        for t in &self.targets {
            let d = normalize_domain(&t.domain);
            if d.is_empty() {
                bail!("found an empty / invalid target domain: '{}'", t.domain);
            }
            if !d.contains('.') {
                log::warn!("target domain '{d}' has no '.' — is this intentional?");
            }
        }

        // Hard fail early with a clear message (no 30-minute hang on missing edges).
        for (label, p) in [
            ("vertices", &self.paths.vertices),
            ("edges", &self.paths.edges),
            ("ranks", &self.paths.ranks),
        ] {
            if !p.exists() {
                bail!(
                    "missing {label} file:\n  {}\n\n\
                     Put the Common Crawl domain graph files next to config.toml, or fix the path in [paths].\n\
                     Need three files: *-domain-vertices.txt, *-domain-edges.txt, *-domain-ranks.txt\n\
                     (download from https://commoncrawl.org/web-graphs )\n\n\
                     For a quick smoke test without multi-GB downloads:\n  .\\run.ps1 -Demo",
                    p.display()
                );
            }
            if p.is_dir() {
                bail!(
                    "{label} path is a directory, not a file:\n  {}\n\
                     Point it at the .txt / .txt.gz file inside.",
                    p.display()
                );
            }
        }
        Ok(())
    }

    /// Resolve config targets to reverse-domain form (deduplicated).
    pub fn resolved_targets(&self) -> Result<Vec<ResolvedTarget>> {
        let mut out = Vec::with_capacity(self.targets.len());
        let mut seen = ahash::AHashSet::new();

        for t in &self.targets {
            let domain = normalize_domain(&t.domain);
            let reverse = to_reverse(&domain);
            if reverse.is_empty() {
                bail!("invalid target domain: '{}'", t.domain);
            }
            if !seen.insert(reverse.clone()) {
                log::warn!("duplicate target domain skipped: {domain}");
                continue;
            }
            out.push(ResolvedTarget { domain, reverse });
        }

        if out.is_empty() {
            bail!("no valid targets after resolution");
        }
        Ok(out)
    }
}

fn resolve_path(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// If `path` is a directory with exactly one file whose name contains `hint`,
/// return that file. Helps when someone extracts an archive into a folder.
fn unwrap_single_file_dir(path: &Path, hint: &str) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let Ok(rd) = fs::read_dir(path) else {
        return path.to_path_buf();
    };
    let mut matches: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_ascii_lowercase().contains(hint))
        })
        .collect();
    if matches.len() == 1 {
        log::info!(
            "using {} inside directory {}",
            matches[0].display(),
            path.display()
        );
        return matches.pop().unwrap();
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            [paths]
            vertices = "v.txt"
            edges = "e.txt"
            ranks = "r.txt"

            [[targets]]
            domain = "https://www.Example.COM/path"
        "#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.config_dir = PathBuf::from(".");
        assert_eq!(cfg.paths.vertices, PathBuf::from("v.txt"));
        assert_eq!(cfg.rank_metric, RankMetric::Pagerank);
        let resolved = cfg.resolved_targets().unwrap();
        assert_eq!(resolved[0].domain, "example.com");
        assert_eq!(resolved[0].reverse, "com.example");
    }
}
