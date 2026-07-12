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

## Build

```bash
# Debug
cargo build

# Optimized release (LTO, stripped)
cargo build --release
```

Binary path:

```
target/release/web-radar        # Linux / macOS
target\release\web-radar.exe    # Windows
```

---

## Configuration

Edit `config.toml`:

```toml
[paths]
vertices = "cc-main-2026-apr-may-jun-domain-vertices.txt"
edges    = "cc-main-2026-apr-may-jun-domain-edges.txt"
ranks    = "cc-main-2026-apr-may-jun-domain-ranks.txt"
results_dir = "results"

# "pagerank" (default) or "harmonic"
rank_metric = "pagerank"

[[targets]]
domain = "example.com"

[[targets]]
domain = "wikipedia.org"
```

Paths may point to plain text or `.gz` files.

---

## Run

```bash
# Default: reads ./config.toml
cargo run --release

# Custom config path
cargo run --release -- --config /path/to/config.toml

# Or after install / direct binary
./target/release/web-radar -c config.toml

# Quiet / verbose logging
./target/release/web-radar -q
./target/release/web-radar -vv      # debug
```

Environment variable `RUST_LOG` is also honoured (`RUST_LOG=debug`).

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
