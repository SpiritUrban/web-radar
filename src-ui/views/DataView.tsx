import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle, ArrowUpRight, Check, Database, FolderOpen, Gauge, Globe2, HardDrive,
  Hammer, Layers, Play, Plus, RefreshCw, Save, Trash2,
} from "lucide-react";
import { hasUnsavedChanges, useAppStore } from "../store";
import { cleanDomain, formatBytes, formatCount, plural } from "../format";
import type { AppConfig, IndexStatus, TierKey, TierStatus } from "../types";

const SOURCE_LABELS: Record<string, { title: string; hint: string }> = {
  vertices: { title: "Vertices", hint: "Список доменів графа — id та назва" },
  edges: { title: "Edges", hint: "Усі зв'язки між доменами; найбільший файл" },
  ranks: { title: "Ranks", hint: "PageRank і Harmonic для кожного домену" },
};

export default function DataView({
  onStatusChange,
  onHistoryChange,
}: {
  onStatusChange: () => void;
  onHistoryChange: () => Promise<void>;
}) {
  const store = useAppStore();
  const [target, setTarget] = useState("");
  const index = store.index;
  const busy = store.busy !== null;
  const unsaved = hasUnsavedChanges(store);

  const choose = async (key: keyof AppConfig, directory: boolean, title: string) => {
    const selected = await open({ directory, multiple: false, title });
    if (typeof selected === "string") {
      store.updateConfig({ [key]: selected } as Partial<AppConfig>);
      onStatusChange();
    }
  };

  const saveConfig = async () => {
    try {
      const path = await invoke<string>("save_config", { config: store.config });
      store.markConfigSaved();
      store.setNotice(`Налаштування збережено у ${path}`);
      onStatusChange();
    } catch (cause) {
      store.setError(String(cause));
    }
  };

  const build = async (tiers: TierKey[]) => {
    store.setBusy("index");
    store.setError(null);
    try {
      const status = await invoke<IndexStatus>("build_index", { config: store.config, tiers });
      store.setIndex(status);
      store.setNotice("Індекс оновлено — запити тепер виконуються миттєво.");
    } catch (cause) {
      store.setError(String(cause));
      onStatusChange();
    } finally {
      store.setBusy(null);
      store.setJob(null);
      await onHistoryChange();
    }
  };

  const dropTier = async (tier: TierKey) => {
    try {
      store.setIndex(await invoke<IndexStatus>("drop_index_tier", { config: store.config, tier }));
    } catch (cause) {
      store.setError(String(cause));
    }
  };

  const fullScan = async () => {
    store.setBusy("scan");
    store.setError(null);
    try {
      const written = await invoke<string[]>("start_full_scan", { config: store.config });
      store.setNotice(`Готово. Записано файлів: ${written.length}. Тека: ${store.config.resultsDir}`);
    } catch (cause) {
      store.setError(String(cause));
    } finally {
      store.setBusy(null);
      store.setJob(null);
      await onHistoryChange();
    }
  };

  const addTarget = () => {
    const value = cleanDomain(target);
    if (!value || store.config.targets.map(cleanDomain).includes(value)) return;
    store.updateConfig({ targets: [...store.config.targets, value] });
    setTarget("");
  };

  const missingTiers = (index?.tiers ?? []).filter((tier) => tier.state !== "ready");
  const totalEstimate = missingTiers.reduce((sum, tier) => sum + tier.estimatedBytes, 0);
  const peakTemp = Math.max(0, ...missingTiers.map((tier) => tier.estimatedTempBytes));
  const notEnoughSpace = index ? index.freeBytes > 0 && index.freeBytes < totalEstimate + peakTemp : false;

  return (
    <section className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <p className="eyebrow"><Database size={14} /> Дані та індекс</p>
          <h2 className="mt-2 text-3xl font-semibold tracking-tight">Файли графа й індексація</h2>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-400">
            Індекс будується один раз і перетворює кожен наступний запит із десятків хвилин
            на мілісекунди. Рівні незалежні — можна побудувати лише те, що потрібно.
          </p>
        </div>
        <div className="flex gap-2">
          <button className="secondary-button" onClick={onStatusChange}>
            <RefreshCw size={15} /> Оновити стан
          </button>
          <button className={unsaved ? "add-button h-10 px-4" : "secondary-button"} onClick={() => void saveConfig()}>
            <Save size={15} /> {unsaved ? "Зберегти зміни" : "Збережено"}
          </button>
        </div>
      </div>

      {index?.blockers.map((blocker) => (
        <div key={blocker} className="flex items-start gap-3 rounded-xl border border-amber-400/25 bg-amber-400/[.07] p-4 text-sm text-amber-100/90">
          <AlertTriangle size={17} className="mt-0.5 shrink-0" />
          <p>{blocker}</p>
        </div>
      ))}

      <div className="grid gap-6 xl:grid-cols-[1.15fr_.85fr]">
        <div className="space-y-6">
          <section className="panel">
            <div className="panel-title">
              <div className="icon-box"><Layers size={18} /></div>
              <div><h3>Файли Common Crawl</h3><p>Три текстові файли графа доменів</p></div>
            </div>
            <div className="mt-6 space-y-4">
              {(["vertices", "edges", "ranks"] as const).map((key) => {
                const source = index?.sources.find((item) => item.kind === key);
                return (
                  <label key={key} className="block">
                    <span className="mb-2 flex items-center justify-between gap-3 text-xs font-semibold uppercase tracking-wider text-slate-400">
                      <span>{SOURCE_LABELS[key].title}</span>
                      <span className={source?.exists ? "text-emerald-400" : "text-rose-400"}>
                        {source?.exists ? formatBytes(source.sizeBytes) : "не знайдено"}
                      </span>
                    </span>
                    <div className="flex gap-2">
                      <input
                        className="input font-mono text-xs"
                        value={store.config[key]}
                        onChange={(event) => store.updateConfig({ [key]: event.target.value } as Partial<AppConfig>)}
                        onBlur={onStatusChange}
                      />
                      <button className="browse-button" type="button" onClick={() => void choose(key, false, `Оберіть файл ${SOURCE_LABELS[key].title}`)}>
                        <FolderOpen size={17} />
                      </button>
                    </div>
                    <span className="mt-1 block text-[11px] text-slate-500">{SOURCE_LABELS[key].hint}</span>
                  </label>
                );
              })}
            </div>
            <p className="mt-5 text-[11px] leading-5 text-slate-500">
              Немає файлів? Завантажте домен-граф з{" "}
              <button
                className="text-cyan-400 hover:underline"
                onClick={() => store.metadata && void openUrl(store.metadata.dataSourceUrl)}
              >
                commoncrawl.org/web-graphs
              </button>{" "}
              — потрібні *-domain-vertices.txt, *-domain-edges.txt і *-domain-ranks.txt.
            </p>
          </section>

          <section className="panel">
            <div className="panel-title">
              <div className="icon-box"><HardDrive size={18} /></div>
              <div>
                <h3>Рівні індексу</h3>
                <p>{index ? `${formatBytes(index.totalBytes)} на диску · вільно ${formatBytes(index.freeBytes)}` : "стан невідомий"}</p>
              </div>
              {missingTiers.length > 0 && (
                <button
                  className="add-button ml-auto h-10 px-4 text-xs"
                  disabled={busy || notEnoughSpace}
                  onClick={() => void build(missingTiers.map((tier) => tier.key))}
                >
                  <Hammer size={15} /> Побудувати все
                </button>
              )}
            </div>

            <div className="mt-3 flex gap-2">
              <input
                className="input font-mono text-xs"
                value={store.config.indexDir}
                onChange={(event) => store.updateConfig({ indexDir: event.target.value })}
                onBlur={onStatusChange}
              />
              <button className="browse-button" type="button" onClick={() => void choose("indexDir", true, "Оберіть теку для індексу")}>
                <FolderOpen size={17} />
              </button>
            </div>

            {notEnoughSpace && (
              <div className="mt-4 flex items-start gap-3 rounded-xl border border-rose-400/25 bg-rose-400/[.07] p-4 text-sm text-rose-100/90">
                <AlertTriangle size={17} className="mt-0.5 shrink-0" />
                <p>
                  Для вибраних рівнів потрібно ≈{formatBytes(totalEstimate + peakTemp)} (з них
                  {" "}{formatBytes(peakTemp)} тимчасово), а вільно {formatBytes(index?.freeBytes ?? 0)}.
                  Вкажіть теку індексу на іншому диску.
                </p>
              </div>
            )}

            <div className="mt-5 space-y-3">
              {(index?.tiers ?? []).map((tier) => (
                <TierRow
                  key={tier.key}
                  tier={tier}
                  busy={busy}
                  onBuild={() => void build([tier.key])}
                  onDrop={() => void dropTier(tier.key)}
                />
              ))}
              {!index && <p className="text-sm text-slate-500">Вкажіть файли графа, щоб побачити стан індексу.</p>}
            </div>

            {index && index.nodeCount > 0 && (
              <div className="mt-5 grid grid-cols-2 gap-3 text-sm">
                <Summary label="Доменів у графі" value={formatCount(index.nodeCount)} />
                <Summary label="Зв'язків у графі" value={formatCount(index.edgeCount)} />
              </div>
            )}
          </section>
        </div>

        <aside className="space-y-6">
          <section className="panel">
            <div className="panel-title">
              <div className="icon-box"><Gauge size={18} /></div>
              <div><h3>Метрика рейтингу</h3><p>Використовується у запитах і експортах</p></div>
            </div>
            <div className="mt-5 grid grid-cols-2 gap-2">
              {(["pagerank", "harmonic"] as const).map((metric) => (
                <button
                  key={metric}
                  className={`metric ${store.config.rankMetric === metric ? "metric-active" : ""}`}
                  onClick={() => store.updateConfig({ rankMetric: metric })}
                >
                  <span className="metric-dot" />{metric === "pagerank" ? "PageRank" : "Harmonic"}
                </button>
              ))}
            </div>
            <p className="mt-3 text-[11px] leading-5 text-slate-500">
              PageRank відображає вагу посилального профілю, Harmonic — близькість домену до решти
              вебу. Обидві зберігаються в індексі, перемикання не потребує перебудови.
            </p>
          </section>

          <section className="panel">
            <div className="panel-title">
              <div className="icon-box"><Globe2 size={18} /></div>
              <div><h3>Домени за замовчуванням</h3><p>Швидкий доступ і повне сканування</p></div>
              <span className="count ml-auto">{store.config.targets.length}</span>
            </div>
            <div className="mt-5 flex gap-2">
              <input
                className="input"
                value={target}
                placeholder="example.com"
                onChange={(event) => setTarget(event.target.value)}
                onKeyDown={(event) => event.key === "Enter" && addTarget()}
              />
              <button className="add-button px-4" onClick={addTarget}><Plus size={17} /></button>
            </div>
            <div className="mt-4 flex flex-wrap gap-2">
              {store.config.targets.map((domain, position) => (
                <span className="domain-chip" key={`${domain}-${position}`}>
                  <Globe2 size={14} />{domain}
                  <button
                    aria-label={`Видалити ${domain}`}
                    onClick={() => store.updateConfig({ targets: store.config.targets.filter((_, i) => i !== position) })}
                  >
                    <Trash2 size={14} />
                  </button>
                </span>
              ))}
              {store.config.targets.length === 0 && (
                <p className="text-sm text-slate-500">Список порожній — для пошуку він не обов'язковий.</p>
              )}
            </div>
          </section>

          <section className="panel">
            <div className="panel-title">
              <div className="icon-box"><Play size={18} /></div>
              <div><h3>Повне сканування</h3><p>Запасний режим без індексу</p></div>
            </div>
            <p className="mt-4 text-[11px] leading-5 text-slate-500">
              Читає всі три файли ({formatBytes((index?.sources ?? []).reduce((sum, source) => sum + source.sizeBytes, 0))})
              за один прохід і записує JSON для кожного домену зі списку вище. Працює й зі
              стиснутими .gz, але триває десятки хвилин — саме це прибирає індекс.
            </p>
            <button
              className="run-button mt-4"
              disabled={busy || store.config.targets.length === 0}
              onClick={() => void fullScan()}
            >
              <Play size={17} fill="currentColor" /> Сканувати {store.config.targets.length}{" "}
              {plural(store.config.targets.length, "домен", "домени", "доменів")}
            </button>
            <button
              className="secondary-button mt-3 w-full justify-center"
              onClick={() => void invoke("open_results_dir", { config: store.config })}
            >
              <ArrowUpRight size={15} /> Відкрити теку результатів
            </button>
          </section>
        </aside>
      </div>
    </section>
  );
}

function TierRow({
  tier, busy, onBuild, onDrop,
}: {
  tier: TierStatus; busy: boolean; onBuild: () => void; onDrop: () => void;
}) {
  const ready = tier.state === "ready";
  const stale = tier.state === "stale";
  return (
    <div className={`rounded-xl border p-4 ${ready ? "border-emerald-400/20 bg-emerald-400/[.04]" : stale ? "border-amber-400/25 bg-amber-400/[.05]" : "border-white/10 bg-white/[.02]"}`}>
      <div className="flex flex-wrap items-center gap-3">
        <span className={`status ${ready ? "status-completed" : stale ? "status-running" : "status-failed"}`}>
          {ready && <Check size={12} />}
          {ready ? "готовий" : stale ? "застарілий" : "не побудований"}
        </span>
        <h4 className="text-sm font-semibold text-slate-200">{tier.label}</h4>
        <span className="text-xs text-slate-500">
          {ready ? formatBytes(tier.bytes) : `≈${formatBytes(tier.estimatedBytes)}`}
          {!ready && tier.estimatedTempBytes > 0 && ` + ${formatBytes(tier.estimatedTempBytes)} тимчасово`}
        </span>
        <div className="ml-auto flex gap-2">
          <button className="secondary-button" disabled={busy} onClick={onBuild}>
            <Hammer size={14} /> {ready ? "Перебудувати" : stale ? "Оновити" : "Побудувати"}
          </button>
          {(ready || stale) && (
            <button className="icon-button h-9 w-9" title="Видалити рівень" disabled={busy} onClick={onDrop}>
              <Trash2 size={15} />
            </button>
          )}
        </div>
      </div>
      <p className="mt-2 text-xs leading-5 text-slate-500">{tier.description}</p>
      {stale && (
        <p className="mt-2 text-xs text-amber-200/80">
          Файли графа змінилися після побудови — відповіді були б із застарілих даних.
        </p>
      )}
    </div>
  );
}

function Summary({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-white/[.06] bg-white/[.02] px-3 py-2">
      <p className="text-[11px] text-slate-500">{label}</p>
      <p className="mt-0.5 font-mono text-sm text-slate-200">{value}</p>
    </div>
  );
}
