//! Web Radar core — indexing and querying Common Crawl domain-level web graphs.
//!
//! The published Common Crawl domain graph is three plain text files:
//!
//! | file | size (2026 apr–jun) | layout |
//! |---|---|---|
//! | `*-domain-vertices.txt` | ~3.4 GB | `id \t reverse_domain \t n_hosts`, sorted by name, `id` = line number |
//! | `*-domain-edges.txt` | ~67 GB | `from_id \t to_id`, sorted by `from_id` then `to_id` |
//! | `*-domain-ranks.txt` | ~8.3 GB | `hc_pos \t hc_val \t pr_pos \t pr_val \t host_rev \t n_hosts`, sorted by `hc_pos` |
//!
//! Answering "who links to this domain?" by streaming all three files costs
//! ~79 GB of reads **per run**. [`index`] turns that into a one-time build,
//! after which [`query`] answers in milliseconds. [`scan`] keeps the original
//! streaming pipeline as the no-index fallback.

pub mod config;
pub mod data_source;
pub mod index;
pub mod meta;
pub mod model;
pub mod progress;
pub mod query;
pub mod reverse;
pub mod scan;

pub use config::{Config, RankMetric};
pub use model::{LinkEntry, TargetResult};
pub use progress::{Cancelled, Progress, ProgressUpdate};
pub use query::{DomainReport, Engine};
