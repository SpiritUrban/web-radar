# Release setup and day-to-day operation

## Cheat sheet — which workflow does what

| I want to… | Do this | What runs |
|---|---|---|
| **Publish a new version** | `npm run version:sync X.Y.Z`, commit, `git tag vX.Y.Z && git push --tags` | `Release`: validate → 4 builds → publish → **deploy the site** |
| **Check the builds without publishing** | Actions → **Release** → Run workflow | validate → 4 builds. `deploy-site` is **skipped on purpose** — a manual run is not a tag |
| **Update only the site** | Actions → **Deploy GitHub Pages** → Run workflow, or push anything under `site/` | manifest → build → deploy |
| **Check code before releasing** | any push or PR | `CI`: types, tests, clippy, fmt, site build |

Two things that look like failures and are not:

* a grey circle on `deploy-site` in a manual `Release` run — that is the skip above;
* a green `Release` run does **not** mean the site was deployed, unless it came from a tag.

> **Do not use "Re-run failed jobs" on `deploy-site`.** Each attempt uploads another
> artifact named `github-pages` into the same run, and `deploy-pages` then refuses:
> *"Multiple artifacts named github-pages were unexpectedly found."* The workflow now
> deletes stale ones first, but that fix only applies from the next tag onward, because
> a re-run uses the workflow file as of the ref that triggered it. Until then, deploy the
> site with **Deploy GitHub Pages → Run workflow** instead — a fresh run, one artifact.

---

## One-time release setup

Everything in this file has to be done **by the repository owner** — an agent cannot do
it. Until it is done the pipeline fails in ways that look unrelated to their cause.

Each step has a **signal**: how to tell it actually worked. An instruction with no way
to verify it is an instruction that fails at the most expensive moment.

---

## Order matters

The `github-pages` environment does not exist until Pages has deployed at least once,
so step 3 is physically impossible before step 2 has succeeded. Work in this order:

**1 → 2 → 4 → 5 → push to `main` → first Pages deploy → 3 → 6 → tag.**

---

### 1. Billing

**Settings → Billing and plans** — an active payment method and a non-zero spending
limit.

*Signal:* jobs start. Without it a job dies in ~3 s with an empty step list and
"The job was not started because recent account payments have failed". Plain CI can
keep working while jobs with an `environment` are blocked, which makes this look like a
Pages problem.

### 2. Pages source

**Settings → Pages → Build and deployment → Source = GitHub Actions.**

*Signal:* after the first push to `main`, `https://spiriturban.github.io/web-radar/`
serves the site.

> ⚠️ **Enabling Pages is not the same as setting its source.** With Pages on but the
> source left at *Deploy from a branch*, GitHub **accepts** the artifact deployment and
> then fails it. What you see: `deploy-pages` runs for its full timeout and reports only
> `Timeout reached, aborting!`, the site answers *Site not found*, and
> `/repos/<owner>/<repo>/deployments` shows `github-pages` deployments ending in
> `failure` — the one from `main` failing in ~30 s, which is a rejection, not a timeout.
>
> Observed on `v0.4.0`: all four builds green, the release complete and correct, and
> only the site missing. `pages.yml` now passes `enablement: true` to
> `actions/configure-pages`, which sets the source itself — but that only helps from the
> *next* tag, because re-running a job uses the workflow file as of the ref that
> triggered it.

### 3. Environment rules (after the first Pages deploy)

**Settings → Environments → github-pages → Deployment branches and tags** — allow the
branch `main` **and** the tag pattern `v*.*.*`.

> ⚠️ The `Add deployment branch or tag rule` dialog has a **Ref type** switch that
> defaults to **Branch**. A `v*.*.*` rule added as a *branch* rule looks right and
> matches nothing — and you find out only after pushing a tag, which must not be moved.

*Signal:* the list header reads **"1 branch and 1 tag allowed"**. If it says
"1 branch and **0 tags**", or `v*.*.*` shows "Currently applies to 0 branches", delete
the rule and re-add it with **Ref type: Tag**.

### 4. Workflow permissions

**Settings → Actions → General → Workflow permissions → Read and write.**

*Signal:* the release job creates a release instead of failing with a 403 on upload.
(`release.yml` also requests `contents: write` per job, so this is belt and braces.)

### 5. Signing keys for auto-update

Generate the key pair locally — it asks for a password interactively, so it cannot be
scripted:

```bash
npm run tauri signer generate -- -w .tauri-key
```

Then:

| where | what |
|---|---|
| **Settings → Secrets and variables → Actions → Repository secrets** → `TAURI_SIGNING_PRIVATE_KEY` | contents of `.tauri-key` (the **private** one) |
| same place → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password you just typed |
| `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` | contents of `.tauri-key.pub` |
| `src-tauri/tauri.conf.json` → `bundle.createUpdaterArtifacts` | change to `true` |

`.tauri-key*` is already in `.gitignore`. **Losing the key or the password is
unrecoverable** — every installed copy stops accepting updates.

*Signal:* the `Validate version and signing config` job passes. It decodes the secret
and checks it is a private rsign key, so pasting the `.pub` file by mistake, or a
trailing newline in the paste, fails in five seconds instead of after a four-minute
compile on all four platforms at once.

> Do **not** set `createUpdaterArtifacts: true` before the secrets exist. Every release
> build would go red. The validation job now says exactly that, but the order still
> matters.

### 6. Dry run before the first tag

`Actions → Release → Run workflow` on `main`. It builds all four platforms and
publishes nothing.

*Signal:* four green build jobs. Note that `deploy-site` is **skipped** on a manual run
by design — so a green dry run means "the builds are fine", not "the release will pass".
That is exactly why step 3 must be done before the tag.

---

## Cutting a release

```bash
npm run version:sync 0.4.0     # updates package.json, Cargo.toml, Cargo.lock, tauri.conf.json
npm run version:check          # proves they agree
git commit -am "release 0.4.0"
git push
git tag v0.4.0 && git push --tags
```

An already-published tag must never be moved. If `deploy-site` fails after a successful
release, fix the environment rule and use
`Actions → Release → <the tag's run> → Re-run failed jobs`: only the site job re-runs,
the builds are not repeated, and `GITHUB_REF_NAME` stays the tag so the manifest asks
for the right release.

---

## Verifying a release without a login

Run logs need authentication even on a public repository; annotations do not.

```bash
curl -s "https://api.github.com/repos/SpiritUrban/web-radar/actions/runs?per_page=3"
curl -s "https://api.github.com/repos/SpiritUrban/web-radar/actions/runs/<RUN_ID>/jobs"
curl -s "https://api.github.com/repos/SpiritUrban/web-radar/check-runs/<JOB_ID>/annotations"

# the updater feed — 11 platform keys for four build jobs
curl -s -L "https://github.com/SpiritUrban/web-radar/releases/download/<TAG>/latest.json" \
  | python -c "import json,sys; d=json.load(sys.stdin); print(len(d['platforms']), sorted(d['platforms']))"

# every download link must answer 206
curl -s -o /dev/null -w '%{http_code}\n' -L -r 0-0 \
  "https://github.com/SpiritUrban/web-radar/releases/download/<TAG>/<FILE>"

# what the live site actually serves
curl -s "https://spiriturban.github.io/web-radar/download-manifest.json"
```

A release is finished only when `latest.json` lists every expected platform: each matrix
job uploads its installers **before** it appends its entries, so "the file downloads"
happens earlier than "the update is available for this platform".

When reading a failed run, look at the **step list** before the error text. Red
`Set up job`, `Post Run …` or `Complete job` means the runner fell over, not the build.
