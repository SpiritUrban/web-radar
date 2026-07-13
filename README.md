# web-radar

Memory-efficient Rust CLI that extracts **inbound and outbound domain links** (plus each target’s own rank) from [Common Crawl](https://commoncrawl.org/) domain-level web graphs.

For every domain listed in `config.toml` the tool streams the multi‑gigabyte graph files (never loads the full edges file into RAM) and writes one JSON report:

1. **Own rank** — PageRank or Harmonic Centrality of the target itself  
2. **Inbound** — domains that link *to* the target (with rank)  
3. **Outbound** — domains the target links *to* (with rank)

Output path:

```text
results/{reversed-domain}.json
```

Example: `skytransfer.com.ua` → `results/ua.com.skytransfer.json`

---

## Requirements

| Need | Notes |
|------|--------|
| [Rust](https://rustup.rs/) 1.75+ | `cargo` on `PATH` |
| Windows PowerShell | for `.\run.ps1` (or run the `.exe` yourself) |
| Disk | vertices ~1–3 GiB, edges ~10–20 GiB, ranks ~1–2 GiB |
| RAM | usually a few hundred MB (depends on neighbor count, not edges file size) |

Install Rust once:

```powershell
# https://rustup.rs — then restart the terminal
rustc --version
cargo --version
```

---

## Quick start (recommended on Windows)

Open PowerShell **in the project root** (`web-radar\`):

```powershell
# 1) Tiny offline demo (no multi-GB downloads) + open results in Explorer
.\run.ps1 -Demo -Open

# 2) Full run against Common Crawl files + open results
.\run.ps1 -Open

# 3) Full run without opening Explorer
.\run.ps1
```

### What `run.ps1` does

1. `cd` to the project folder (safe if you launch it from elsewhere)  
2. `cargo build --release` (unless `-SkipBuild`)  
3. Runs `target\release\web-radar.exe -c …`  
4. Prints **absolute paths** to every `*.json` under the results folder  
5. Optionally opens that folder in Explorer (`-Open`)

### `run.ps1` flags

| Flag | Meaning |
|------|---------|
| *(none)* | Build + run with `config.toml` → results in `results\` |
| `-Demo` | Use tiny fixture `testdata\config.toml` → `testdata\results\` (no CC downloads) |
| `-Open` | After a successful run, open the results folder in Explorer |
| `-SkipBuild` | Skip `cargo build`; use an already-built `target\release\web-radar.exe` |

Examples:

```powershell
.\run.ps1 -Demo              # quick smoke test
.\run.ps1 -Demo -Open        # smoke test + open folder
.\run.ps1 -Open              # real graph run + open folder
.\run.ps1 -SkipBuild -Open   # re-run without recompiling
```

If PowerShell blocks scripts:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
# or one-shot:
powershell -ExecutionPolicy Bypass -File .\run.ps1 -Demo -Open
```

---

## Full run checklist (real Common Crawl data)

1. **Download** a domain graph from [Common Crawl Web Graphs](https://commoncrawl.org/web-graphs). You need three files:

   | Kind | Typical name |
   |------|----------------|
   | Vertices | `*-domain-vertices.txt` (or `.gz`) |
   | Edges | `*-domain-edges.txt` (or `.gz`, largest) |
   | Ranks | `*-domain-ranks.txt` (or `.gz`) |

2. **Place them** next to `config.toml` (or point `[paths]` at them).  
   Folders that contain a single matching file are OK — the tool picks the file inside automatically.

3. **Edit** `config.toml` — paths + target domains:

```toml
[paths]
vertices = "cc-main-2026-apr-may-jun-domain-vertices.txt"
edges    = "cc-main-2026-apr-may-jun-domain-edges.txt"
ranks    = "cc-main-2026-apr-may-jun-domain-ranks.txt"
results_dir = "results"

rank_metric = "pagerank"   # or "harmonic"

[[targets]]
domain = "https://example.com/"

[[targets]]
domain = "https://another-site.com.ua/"
```

4. **Run** from the project root:

```powershell
.\run.ps1 -Open
```

A full edges pass can take a long time (tens of minutes on HDD; faster on SSD). Progress bars show throughput and ETA.

5. **Read** results under `results\` (absolute path is printed at the end).

---

## Configuration reference

Paths in `config.toml` are **relative to the config file’s directory** (not necessarily your current working directory).

| Key | Description |
|-----|-------------|
| `paths.vertices` | Domain vertices file or folder containing it |
| `paths.edges` | Domain edges file or folder |
| `paths.ranks` | Domain ranks file or folder |
| `paths.results_dir` | Output directory (default `results`) |
| `rank_metric` | `"pagerank"` (default) or `"harmonic"` |
| `[[targets]].domain` | URL or bare hostname — `https://`, `www.`, paths, ports are stripped |

Gzip (`.gz`) is detected by extension and streamed automatically.

---

## Manual build & run (without `run.ps1`)

```powershell
# from project root
cargo build --release

# real config
.\target\release\web-radar.exe -c config.toml

# demo fixture
.\target\release\web-radar.exe -c testdata\config.toml
```

### CLI flags

| Flag | Description |
|------|-------------|
| `-c`, `--config <FILE>` | Config path (default `config.toml`) |
| `-v` / `-vv` / `-vvv` | More log detail (`info` / `debug` / `trace`) |
| `-q`, `--quiet` | Errors only |

Logging also respects `RUST_LOG` if you set it, e.g.:

```powershell
$env:RUST_LOG = "debug"
.\target\release\web-radar.exe -c config.toml
```

---

## Output schema

### Found domain

```json
{
  "domain": "example.com",
  "found": true,
  "rank": 0.05,
  "inbound": [
    { "domain": "other.com", "rank": 123.45 },
    { "domain": "blog.news.org", "rank": 12.3 }
  ],
  "outbound": [
    { "domain": "partner.org", "rank": 9.1 },
    { "domain": "cdn.assets.net", "rank": 0.4 }
  ]
}
```

`inbound` and `outbound` are sorted by `rank` **descending** (strongest first).

### Not found in the graph (stub)

If a target is missing from Common Crawl vertices, a stub is still written so the gap is obvious:

```json
{
  "domain": "my-transfer.com.ua",
  "found": false,
  "rank": null,
  "inbound": [],
  "outbound": []
}
```

### Field meanings

| Field | Meaning |
|-------|---------|
| `domain` | Normalized target hostname |
| `found` | `true` if the domain exists in vertices; `false` → stub only |
| `rank` | Centrality of the **target itself** (`null` if not found) |
| `inbound[]` | Who links **to** the target |
| `outbound[]` | Where the target links **to** |
| `*.rank` | Same metric as `rank_metric` for that neighbor |

**Filename** = reverse domain of the target:

| Domain | File |
|--------|------|
| `example.com` | `com.example.json` |
| `skytransfer.com.ua` | `ua.com.skytransfer.json` |

---

## Understanding `rank`

- Comes from Common Crawl ranks (`#pr_val` or `#harmonicc_val`).
- **Larger number = more important** in the link graph (not Google SERP position).
- Values are small fractions (often `0.01` for global giants, `1e-9` for small sites).  
  Example: `7.97e-9` means `0.00000000797` — **much smaller** than `0.009`.
- Do not compare PageRank and Harmonic numbers directly (different scales).

---

## How it works

Four streaming passes:

1. **Vertices (targets)** — map configured domains → node IDs. Missing domains get `found: false` stubs immediately.  
2. **Edges** — single pass: inbound (`to` = target) + outbound (`from` = target).  
3. **Vertices (neighbors)** — resolve neighbor IDs → domain names.  
4. **Ranks** — attach centrality to targets and neighbors.

Progress bars ([indicatif](https://crates.io/crates/indicatif)) show bytes / speed / ETA per pass.

---

## Project layout

```text
web-radar/
├── Cargo.toml
├── config.toml              # real run configuration
├── run.ps1                  # one-command build + run (Windows)
├── README.md
├── results/                 # JSON output (full run)
├── testdata/                # tiny offline demo
│   ├── config.toml
│   ├── vertices.txt
│   ├── edges.txt
│   ├── ranks.txt
│   └── results/
└── src/
    ├── main.rs              # CLI (clap + logging)
    ├── config.rs            # TOML load & validation
    ├── reverse.rs           # reverse-domain helpers
    └── processor.rs         # multi-pass pipeline
```

Graph dumps (`*-domain-*.txt`) live next to `config.toml` when you do a full run (often as a folder with the file inside).

---

## Troubleshooting

| Problem | What to do |
|---------|------------|
| `config file not found` | Run from project root, or pass full path: `-c C:\path\to\config.toml`. Prefer `.\run.ps1`. |
| `missing vertices/edges/ranks file` | Download the three domain-graph files and fix `[paths]` in `config.toml`. Or try `.\run.ps1 -Demo`. |
| `found: false` for a domain | Domain is not in this CC release (too new / rarely crawled). Not a code bug. |
| Script won’t run | `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` or `powershell -ExecutionPolicy Bypass -File .\run.ps1` |
| Slow full run | Use SSD + release build; decompress `.gz` to `.txt` if CPU-bound on gzip. |
| `cargo` not found | Install [Rust](https://rustup.rs/) and open a **new** terminal. |

---

## Performance tips

- Always use **release**: `cargo build --release` / `.\run.ps1`.  
- Keep graph files on a fast local SSD.  
- Prefer plain `.txt` over `.gz` if you have disk (less CPU).  
- Limit `[[targets]]` — popular sites (e.g. `google.com`) create huge neighbor sets.  
- Peak RAM tracks unique neighbors for your targets, not the full edges file size.

---

## License

MIT
