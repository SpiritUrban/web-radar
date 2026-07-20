import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  Activity, ArrowUpRight, Check, Database, FileJson, FolderOpen,
  Gauge, Globe2, History, LoaderCircle, Play, Plus, Radar, RefreshCw,
  Settings2, Trash2, TriangleAlert,
} from "lucide-react";
import { useAppStore } from "./store";
import type { AppConfig, FileIndex, ProgressEvent, RunRecord } from "./types";

const formatBytes = (bytes: number) => {
  if (!bytes) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** power).toFixed(power > 1 ? 1 : 0)} ${units[power]}`;
};

function App() {
  const store = useAppStore();
  const [target, setTarget] = useState("");
  const [view, setView] = useState<"setup" | "history">("setup");

  const refresh = async () => {
    const [config, history, files] = await Promise.all([
      invoke<AppConfig>("load_default_config"),
      invoke<RunRecord[]>("get_run_history"),
      invoke<FileIndex[]>("get_file_index"),
    ]);
    store.setConfig(config);
    store.setHistory(history);
    store.setFiles(files);
  };

  useEffect(() => {
    refresh().catch((e) => store.setError(String(e)));
    const unlisten = listen<ProgressEvent>("run-progress", ({ payload }) => store.setProgress(payload));
    return () => { void unlisten.then((fn) => fn()); };
  }, []);

  const addTarget = () => {
    const value = target.trim();
    if (!value || store.config.targets.includes(value)) return;
    store.updateConfig({ targets: [...store.config.targets, value] });
    setTarget("");
  };

  const choosePath = async (key: "vertices" | "edges" | "ranks" | "resultsDir") => {
    const selected = await open({
      directory: key === "resultsDir",
      multiple: false,
      title: key === "resultsDir" ? "Оберіть папку результатів" : "Оберіть файл Common Crawl",
    });
    if (typeof selected === "string") store.updateConfig({ [key]: selected });
  };

  const run = async () => {
    store.setError(null);
    store.setRunning(true);
    store.setProgress({ runId: 0, phase: "starting", message: "Підготовка…", progress: 0 });
    try {
      await invoke("start_analysis", { config: store.config });
      const [history, files] = await Promise.all([
        invoke<RunRecord[]>("get_run_history"),
        invoke<FileIndex[]>("get_file_index"),
      ]);
      store.setHistory(history);
      store.setFiles(files);
    } catch (e) {
      store.setError(String(e));
    } finally {
      store.setRunning(false);
    }
  };

  const validTargets = useMemo(() => store.config.targets.filter((x) => x.trim()).length, [store.config.targets]);

  return (
    <div className="min-h-screen text-slate-100">
      <header className="border-b border-white/10 bg-slate-950/75 backdrop-blur-xl">
        <div className="mx-auto flex max-w-[1440px] items-center justify-between px-7 py-4">
          <div className="flex items-center gap-3">
            <div className="grid h-10 w-10 place-items-center rounded-xl bg-cyan-400 text-slate-950 shadow-lg shadow-cyan-400/20"><Radar size={24}/></div>
            <div><h1 className="text-lg font-bold tracking-tight">Web Radar</h1><p className="text-xs text-slate-400">Common Crawl link intelligence</p></div>
          </div>
          <nav className="flex rounded-xl border border-white/10 bg-white/[0.04] p-1 text-sm">
            <button className={view === "setup" ? "nav-active" : "nav-button"} onClick={() => setView("setup")}><Settings2 size={15}/> Аналіз</button>
            <button className={view === "history" ? "nav-active" : "nav-button"} onClick={() => setView("history")}><History size={15}/> Історія <span className="badge">{store.history.length}</span></button>
          </nav>
        </div>
      </header>

      <main className="mx-auto max-w-[1440px] px-7 py-8">
        {view === "setup" ? (
          <>
            <section className="mb-8 flex items-end justify-between gap-6">
              <div><p className="eyebrow"><Activity size={14}/> Нова операція</p><h2 className="mt-2 text-3xl font-semibold tracking-tight">Дослідіть зв’язки доменів</h2><p className="mt-2 max-w-2xl text-sm leading-6 text-slate-400">Задайте локальні файли графа, додайте цільові домени та отримайте їхні вхідні й вихідні зв’язки.</p></div>
              <button className="secondary-button" onClick={() => refresh().catch((e) => store.setError(String(e)))}><RefreshCw size={16}/> Повернути config.toml</button>
            </section>

            {store.error && <div className="mb-6 flex items-start gap-3 rounded-xl border border-rose-400/30 bg-rose-400/10 p-4 text-sm text-rose-200"><TriangleAlert size={18}/><div><b>Не вдалося виконати операцію</b><p className="mt-1 whitespace-pre-wrap text-rose-200/80">{store.error}</p></div></div>}

            <div className="grid gap-6 xl:grid-cols-[1.3fr_.7fr]">
              <div className="space-y-6">
                <section className="panel">
                  <div className="panel-title"><div className="icon-box"><Database size={18}/></div><div><h3>Файли графа</h3><p>Початкові значення завантажені з config.toml</p></div></div>
                  <div className="mt-6 space-y-4">
                    {(["vertices", "edges", "ranks"] as const).map((key) => (
                      <PathField key={key} label={{vertices:"Vertices", edges:"Edges", ranks:"Ranks"}[key]} value={store.config[key]} onChange={(value) => store.updateConfig({[key]: value})} onBrowse={() => choosePath(key)} index={store.files.find((f) => f.kind === key)}/>
                    ))}
                    <PathField label="Папка результатів" value={store.config.resultsDir} onChange={(resultsDir) => store.updateConfig({resultsDir})} onBrowse={() => choosePath("resultsDir")}/>
                  </div>
                </section>

                <section className="panel">
                  <div className="panel-title"><div className="icon-box"><Globe2 size={18}/></div><div><h3>Цільові домени</h3><p>URL або hostname — формат нормалізується автоматично</p></div><span className="ml-auto count">{validTargets}</span></div>
                  <div className="mt-6 flex gap-2"><input className="input" value={target} onChange={(e) => setTarget(e.target.value)} onKeyDown={(e) => e.key === "Enter" && addTarget()} placeholder="https://example.com"/><button className="add-button" onClick={addTarget}><Plus size={18}/> Додати</button></div>
                  <div className="mt-4 flex flex-wrap gap-2">
                    {store.config.targets.map((domain, i) => <span className="domain-chip" key={`${domain}-${i}`}><Globe2 size={14}/>{domain}<button aria-label={`Видалити ${domain}`} onClick={() => store.updateConfig({targets: store.config.targets.filter((_, n) => n !== i)})}><Trash2 size={14}/></button></span>)}
                    {!store.config.targets.length && <p className="text-sm text-slate-500">Додайте принаймні один домен.</p>}
                  </div>
                </section>
              </div>

              <aside className="space-y-6">
                <section className="panel">
                  <div className="panel-title"><div className="icon-box"><Gauge size={18}/></div><div><h3>Параметри запуску</h3><p>Метрика ранжування</p></div></div>
                  <div className="mt-5 grid grid-cols-2 gap-2">
                    {(["pagerank", "harmonic"] as const).map((metric) => <button key={metric} className={`metric ${store.config.rankMetric === metric ? "metric-active" : ""}`} onClick={() => store.updateConfig({rankMetric: metric})}><span className="metric-dot"/>{metric === "pagerank" ? "PageRank" : "Harmonic"}</button>)}
                  </div>
                  <div className="my-6 h-px bg-white/10"/>
                  <div className="space-y-3 text-sm">
                    <SummaryRow label="Доменів" value={String(validTargets)}/>
                    <SummaryRow label="Формат" value="JSON"/>
                    <SummaryRow label="Обробка" value="Rayon + Tokio"/>
                    <SummaryRow label="Історія" value="SQLite"/>
                  </div>
                  {store.running && <div className="mt-6 rounded-xl border border-cyan-400/20 bg-cyan-400/[0.07] p-4"><div className="flex items-center gap-2 text-sm font-medium text-cyan-200"><LoaderCircle className="animate-spin" size={16}/>{store.progress?.message ?? "Обробка…"}</div><div className="mt-3 h-1.5 overflow-hidden rounded-full bg-slate-800"><div className="h-full rounded-full bg-cyan-400 transition-all" style={{width:`${Math.max(4, (store.progress?.progress ?? 0) * 100)}%`}}/></div></div>}
                  <button className="run-button mt-6" disabled={store.running || validTargets === 0} onClick={run}>{store.running ? <LoaderCircle className="animate-spin" size={19}/> : <Play size={19} fill="currentColor"/>}{store.running ? "Аналізуємо…" : "Запустити аналіз"}</button>
                  <p className="mt-3 text-center text-xs text-slate-500">Великі графи можуть оброблятися десятки хвилин</p>
                </section>
                <section className="panel">
                  <div className="flex items-center justify-between"><div className="panel-title"><div className="icon-box"><FileJson size={18}/></div><div><h3>Результати</h3><p>{store.config.resultsDir || "Не вибрано"}</p></div></div><button className="icon-button" title="Відкрити папку" onClick={() => openPath(store.config.resultsDir)}><ArrowUpRight size={17}/></button></div>
                </section>
              </aside>
            </div>
          </>
        ) : <HistoryView history={store.history} />}
      </main>
    </div>
  );
}

function PathField({label, value, onChange, onBrowse, index}:{label:string; value:string; onChange:(v:string)=>void; onBrowse:()=>void; index?:FileIndex}) {
  return <label className="block"><span className="mb-2 flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-slate-400">{label}{index && <span className={index.exists ? "text-emerald-400" : "text-rose-400"}>{index.exists ? formatBytes(index.sizeBytes) : "не знайдено"}</span>}</span><div className="flex gap-2"><input className="input font-mono text-xs" value={value} onChange={(e)=>onChange(e.target.value)}/><button className="browse-button" onClick={onBrowse} type="button"><FolderOpen size={17}/></button></div></label>;
}

function SummaryRow({label,value}:{label:string;value:string}) { return <div className="flex justify-between"><span className="text-slate-400">{label}</span><span className="font-medium text-slate-200">{value}</span></div>; }

function HistoryView({history}:{history:RunRecord[]}) {
  return <section><p className="eyebrow"><History size={14}/> Локальна база</p><h2 className="mt-2 text-3xl font-semibold">Історія операцій</h2><div className="panel mt-8 overflow-hidden p-0"><div className="grid grid-cols-[80px_1fr_150px_120px_100px] gap-4 border-b border-white/10 px-6 py-3 text-xs font-semibold uppercase tracking-wider text-slate-500"><span>ID</span><span>Запуск</span><span>Метрика</span><span>Доменів</span><span>Статус</span></div>{history.length ? history.map((run)=><div key={run.id} className="grid grid-cols-[80px_1fr_150px_120px_100px] items-center gap-4 border-b border-white/[0.06] px-6 py-4 text-sm last:border-0"><span className="font-mono text-slate-500">#{run.id}</span><div><p>{new Date(run.startedAt).toLocaleString("uk-UA")}</p>{run.error && <p className="mt-1 truncate text-xs text-rose-400">{run.error}</p>}</div><span className="capitalize text-slate-400">{run.rankMetric}</span><span>{run.targets.length}</span><span className={`status status-${run.status}`}>{run.status === "completed" && <Check size={12}/>} {run.status}</span></div>) : <div className="p-12 text-center text-sm text-slate-500">Історія з’явиться після першого запуску.</div>}</div></section>;
}

export default App;
