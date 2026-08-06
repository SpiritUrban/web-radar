# Web Radar

**Who links to this domain?** — answered in milliseconds, from the
[Common Crawl](https://commoncrawl.org/web-graphs) domain-level web graph, entirely on your machine.

The published graph is three plain text files. For the 2026 apr–jun crawl that is
**79 GB** covering **121 091 933 domains** and **3.9 billion links**:

| file | size | layout |
|---|---|---|
| `*-domain-vertices.txt` | 3.4 GB | `id ⇥ reverse_domain ⇥ n_hosts`, sorted by name |
| `*-domain-edges.txt` | 67 GB | `from_id ⇥ to_id`, sorted by source |
| `*-domain-ranks.txt` | 8.3 GB | harmonic and PageRank positions and values |

Answering one backlink question by streaming those files costs a full read of all
79 GB — every single time. Web Radar reads them **once**, into an index, and every
question after that is a seek.

| | streaming (v0.2, still available) | with the index (v0.3) |
|---|---|---|
| find a domain | full scan of vertices | **instant** |
| where it links to | full scan of edges (67 GB) | **instant** |
| who links to it | full scan of edges (67 GB) | **instant** |
| rank + global position | full scan of ranks | **instant**, position included |
| measured on the graph above | tens of minutes per run | **14–38 ms per query** |

Web Radar ships as a desktop app (Windows, macOS, Linux) and as a CLI. Same engine,
same index, no account, no network.

---

## Install

Download an installer from the [releases page](https://github.com/SpiritUrban/web-radar/releases)
or the [project site](https://spiriturban.github.io/web-radar/). Installed copies update
themselves.

Building from source needs [Rust](https://rustup.rs/) 1.80+ and Node 20+:

```bash
npm ci
npm run tauri build     # desktop app
cargo build --release   # CLI only, at target/release/web-radar
```

---

## Get the data

Web Radar does not download the graph for you — it is ~16 GB from a third party, and a
silent multi-hour download inside an app is worse than an honest instruction. The app
shows the exact links, sizes and target folder on its **Дані та індекс** tab, and
`web-radar index status` prints the same thing.

Direct links for the reference crawl (all three must come from the *same* release):

```text
https://data.commoncrawl.org/projects/hyperlinkgraph/cc-main-2026-apr-may-jun/domain/cc-main-2026-apr-may-jun-domain-vertices.txt.gz   838 MB → unpack
https://data.commoncrawl.org/projects/hyperlinkgraph/cc-main-2026-apr-may-jun/domain/cc-main-2026-apr-may-jun-domain-edges.txt.gz    13.3 GB → unpack
https://data.commoncrawl.org/projects/hyperlinkgraph/cc-main-2026-apr-may-jun/domain/cc-main-2026-apr-may-jun-domain-ranks.txt.gz     2.2 GB → leave gzipped
```

A new release comes out every two to three months; the crawl id is the only part of the
URL that changes, and newer ones are listed at
[commoncrawl.org/web-graphs](https://commoncrawl.org/web-graphs). Replace the files and
the app marks the affected index tiers **stale** on its own.

Point `config.toml` at them (a folder containing the file works too):

```toml
[paths]
vertices = "cc-main-2026-apr-may-jun-domain-vertices.txt"
edges    = "cc-main-2026-apr-may-jun-domain-edges.txt"
ranks    = "cc-main-2026-apr-may-jun-domain-ranks.txt"
results_dir = "results"

# The index can reach ~17 GB and needs ~30 GB of temporary space while the
# backlink tier is built. Point it at a drive with room.
index_dir = "D:/web-radar-index"

rank_metric = "pagerank"   # or "harmonic"

[[targets]]
domain = "https://example.com/"
```

> Keep the graph files out of a synced folder (OneDrive, Dropbox). They are tens of
> gigabytes and every pass over them will fight the sync client.

---

## Build the index

Three independent tiers — build only what you need, drop any of them to reclaim space.
Times below are measured on the graph above, on a 4-core laptop with a ~100 MB/s volume:

| tier | what it enables | size | build time |
|---|---|---|---|
| `lookup` | find a domain, list where it links **to** | 17 MB | 35 s |
| `ranks` | PageRank / harmonic **and global position** for every domain | 1.8 GB | ~10 min |
| `inbound` | who links **to** a domain — the backlink question | ~15 GB | ~30–50 min, ~30 GB temp |

```bash
web-radar index status          # what exists, what it would cost
web-radar index build lookup    # start here — a minute, and search works
web-radar index build           # everything
web-radar index drop inbound    # reclaim the big one
```

In the desktop app this is the **Дані та індекс** tab: the same tiers with a progress
bar, measured throughput, an ETA and a working Cancel button.

The build refuses to start if the target volume does not have room, naming the number
it needs.

**Only `vertices` and `edges` have to be unpacked.** Queries seek into those two, and
gzip cannot be seeked. `ranks` is read once, sequentially, and never again — so leave it
as the `.gz` you downloaded and save 6 GB of disk and one unpacking step.

---

## Use it

```bash
web-radar query example.com                 # table
web-radar query example.com --json --save   # results/com.example.json
web-radar query example.com --metric harmonic --top 50
```

```text
skytransfer.com.ua
  pagerank: 4.547172e-9 (#26892560)
  inbound: 10   outbound: 15   in 14 ms
```

Every answer says what it does **not** know. Without the `ranks` tier ranks read as 0
and the app says so; without `inbound` it says the backlink question is unanswered
rather than reporting zero backlinks.

### Without an index

`web-radar run` keeps the original v0.2 pipeline: one streaming pass over all three
files for every domain in `config.toml`, writing `results/{reversed-domain}.json`. It
needs no extra disk, works on `.gz`, and takes tens of minutes. The desktop app exposes
it as **Повне сканування**.

---

## How the index works

The three files have properties that make an index cheap, and the build **verifies**
each one instead of assuming it — an unsorted file disables the affected lookup rather
than producing wrong answers.

* **vertices is sorted by domain, and `id` is the line number.** So both directions are
  binary-searchable. A sparse block index (one entry per 256 lines, ~19 MB for 121 M
  domains) turns any lookup into one in-memory binary search plus a single ~7 KB read.
* **edges is sorted by source.** All links from one domain are contiguous, so outbound
  needs only the id at a few thousand file positions — the edges index is built by
  *sampling* every 4 MB, not by reading 67 GB.
* **ranks is sorted by centrality, not by domain**, so it cannot be searched at all. It
  is joined against the vertex index once and stored as a flat array addressed by id
  (16 bytes per domain: both values, both positions).
* **backlinks require transposing 3.9 billion edges**, which does not fit in RAM. An
  external bucket sort writes `(to, from)` pairs into id-range buckets, sorts each
  bucket, and appends it to a CSR structure (`inbound.off` offsets + `inbound.src`
  sources). That is the ~30 GB of temporary space, and it is released as it goes.

---

## Repository layout

```
crates/core/     engine: config, index build/read, query, streaming scan
crates/cli/      the `web-radar` binary
src-tauri/       desktop shell (Tauri v2) — commands, SQLite history, SEO tools
src-ui/          desktop UI (React + Tailwind)
site/            marketing site published to GitHub Pages
scripts/         version sync, download manifest, CI annotations
```

One Cargo workspace, one `Cargo.lock`, one version — `npm run version:sync 0.4.0`
updates every file and `npm run version:check` (run by CI) proves they agree.

```bash
cargo test --workspace     # engine
npx vitest run             # UI and scripts
cargo clippy --workspace --all-targets -- -D warnings
```

Releases, the update feed and the site are automated — see [SETUP.md](SETUP.md) for the
one-time GitHub configuration the repository owner has to do by hand.

---

## Author

Built by **[Vitaliy Dyachuk](https://spiriturban.github.io/)** — engineer working on
tools for large-scale data and the web. More projects and services on the
[personal hub](https://spiriturban.github.io/).

Data: [Common Crawl Foundation](https://commoncrawl.org/). Licensed under
[MIT](LICENSE) — free to use, keep the attribution.
