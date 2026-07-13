# web-radar

Memory-efficient Rust CLI that extracts **inbound domain links** from [Common Crawl](https://commoncrawl.org/) domain-level web graphs.

For each target domain in `config.toml` the tool scans the multi-gigabyte edges file (streamed via `BufReader`, never loaded fully into RAM), finds every domain that links *to* the target, attaches a rank (PageRank or Harmonic Centrality), and writes:

```
results/{reversed-domain}.json
```

Example output (`results/com.example.json`):

```json
[
  {
    "source": "other.com",
    "rank": 123.45
  },
  {
    "source": "blog.news.org",
    "rank": 12.3
  }
]
```

Sources are sorted by `rank` descending.

---

## Common Crawl input format

Download a domain graph release from the [Common Crawl Web Graphs](https://commoncrawl.org/web-graphs) page. You need three files:

| File | Format | Description |
|------|--------|-------------|
| `*-domain-vertices.txt[.gz]` | `id \t rev_domain \t n_hosts` | Nodes in reverse domain notation |
| `*-domain-edges.txt[.gz]` | `from_id \t to_id` | Directed edges (~10–20 GiB) |
| `*-domain-ranks.txt[.gz]` | `#harmonicc_pos \t #harmonicc_val \t #pr_pos \t #pr_val \t #host_rev \t …` | Centrality ranks |

Domains are stored in **reverse domain notation**: `example.com` → `com.example`.

Gzip-compressed files (`.gz`) are detected automatically.

---

## Requirements

- [Rust](https://rustup.rs/) 1.75+ (edition 2021)
- Disk space for the graph files (vertices ~1 GiB, edges ~13+ GiB, ranks ~2 GiB)
- RAM: typically a few hundred MB depending on how many unique sources link to your targets (not proportional to the edges file size)

---

## Quick start (Windows, least hassle)

From the project folder in PowerShell:

```powershell
# Tiny demo (no multi-GB downloads) + open results folder
.\run.ps1 -Demo -Open

# Real run (needs vertices + edges + ranks files next to config.toml)
.\run.ps1 -Open
```

`run.ps1` builds the release binary if needed, runs the tool, then prints
**absolute paths** to every JSON file. Results always go under:

```
web-radar\results\                 # normal run
web-radar\testdata\results\        # -Demo
```

---

## Build (manual)

```bash
cargo build --release
```

Binary: `target/release/web-radar.exe` (Windows) or `target/release/web-radar`.

---

## Configuration

Edit `config.toml` (paths are relative to the config file):

```toml
[paths]
vertices = "cc-main-2026-apr-may-jun-domain-vertices.txt"
edges    = "cc-main-2026-apr-may-jun-domain-edges.txt"
ranks    = "cc-main-2026-apr-may-jun-domain-ranks.txt"
results_dir = "results"

rank_metric = "pagerank"   # or "harmonic"

[[targets]]
domain = "https://example.com/"   # URL or bare domain — both OK
```

You need **all three** graph files. Download from
[Common Crawl Web Graphs](https://commoncrawl.org/web-graphs).

---

## Run (manual)

```powershell
# always from project root
.\target\release\web-radar.exe -c config.toml

# demo fixture
.\target\release\web-radar.exe -c testdata\config.toml
```

`RUST_LOG=debug` works if you want more noise.

---

## How it works

Four streaming passes keep peak memory low:

1. **Vertices (targets)** — map each configured domain to its numeric node ID.
2. **Edges (stream)** — for every `from → to` edge, if `to` is a target, record `from` as an inbound source. The edges file is read line-by-line with a 1 MiB buffer; nothing is held except the collected source ID sets.
3. **Vertices (sources)** — resolve collected source IDs back to reverse domain names.
4. **Ranks** — attach PageRank / Harmonic Centrality to those sources.

Then one pretty-printed JSON file per target is written under `results/`.

Progress bars ([indicatif](https://crates.io/crates/indicatif)) show throughput and ETA for each pass.

---

## Project layout

```
web-radar/
├── Cargo.toml
├── config.toml          # example configuration
├── README.md
└── src/
    ├── main.rs          # CLI entry point (clap + logging)
    ├── config.rs        # TOML config types & validation
    ├── reverse.rs       # reverse domain notation helpers
    └── processor.rs     # multi-pass streaming pipeline
```

---

## Output schema

```json
[
  { "source": "<normal-domain>", "rank": <float> }
]
```

| Field | Meaning |
|-------|---------|
| `source` | Domain that links to the target (normal notation) |
| `rank` | PageRank (`#pr_val`) or Harmonic Centrality (`#harmonicc_val`), depending on `rank_metric` |

Filename = reverse domain of the target, e.g. `wikipedia.org` → `results/org.wikipedia.json`.

---

## Performance tips

- Prefer **release** builds (`cargo build --release`).
- Keep graph files on a fast local SSD.
- Using pre-decompressed `.txt` files avoids gzip CPU overhead at the cost of more disk.
- Limit `[[targets]]` to domains you care about — source-ID sets grow with popularity of the target (e.g. `google.com` has many inbound edges).
- For extremely popular targets, ensure free RAM is enough to hold the unique source ID set plus their names/ranks (usually still far below loading the edges file).

---

## License

MIT
