//! Product and author metadata — defined once, used by the CLI, the desktop
//! app, the installer bundle and the website.

/// Human-facing product name (matches `productName` in `tauri.conf.json`).
pub const PRODUCT_NAME: &str = "Web Radar";
/// Author's real name — also the name in `LICENSE`.
pub const AUTHOR: &str = "Vitaliy Dyachuk";
/// Personal hub: who the author is, and what else he builds.
pub const AUTHOR_URL: &str = "https://spiriturban.github.io/";
pub const AUTHOR_GITHUB_URL: &str = "https://github.com/SpiritUrban";
pub const REPOSITORY_URL: &str = "https://github.com/SpiritUrban/web-radar";
pub const SITE_URL: &str = "https://spiriturban.github.io/web-radar/";
pub const COPYRIGHT: &str = "© 2026 Vitaliy Dyachuk";
/// Where the source graph files come from.
pub const DATA_SOURCE_URL: &str = "https://commoncrawl.org/web-graphs";

/// Version of the crate, i.e. of the whole workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
