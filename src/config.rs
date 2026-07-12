//! Configuration loading from `config.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::reverse::to_reverse;

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
}

/// Resolved target ready for processing.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    /// Original domain from config (`example.com`).
    pub domain: String,
    /// Reverse domain notation (`com.example`).
    pub reverse: String,
}

impl Config {
    /// Load and validate config from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.targets.is_empty() {
            bail!("config has no [[targets]]; add at least one domain");
        }
        for t in &self.targets {
            let d = t.domain.trim();
            if d.is_empty() {
                bail!("found an empty target domain in config");
            }
            if !d.contains('.') {
                log::warn!(
                    "target domain '{d}' has no '.' — is this intentional?"
                );
            }
        }
        for (label, p) in [
            ("vertices", &self.paths.vertices),
            ("edges", &self.paths.edges),
            ("ranks", &self.paths.ranks),
        ] {
            if !p.exists() {
                log::warn!(
                    "configured {label} path does not exist yet: {}",
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
            let domain = t.domain.trim().to_ascii_lowercase();
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
            domain = "Example.COM"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.paths.vertices, PathBuf::from("v.txt"));
        assert_eq!(cfg.rank_metric, RankMetric::Pagerank);
        let resolved = cfg.resolved_targets().unwrap();
        assert_eq!(resolved[0].reverse, "com.example");
    }
}
