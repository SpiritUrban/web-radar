import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Database, ExternalLink, FileSearch, History, Info, LoaderCircle, Radar, Search,
  Sparkles, TriangleAlert, X,
} from "lucide-react";
import { useAppStore } from "./store";
import SearchView from "./views/SearchView";
import DataView from "./views/DataView";
import HistoryView from "./views/HistoryView";
import AboutView from "./views/AboutView";
import SeoDiscoveryView from "./views/SeoDiscoveryView";
import ManualResearchView from "./views/ManualResearchView";
import UpdateBanner from "./components/UpdateBanner";
import JobBar from "./components/JobBar";
import type { AppConfig, IndexStatus, JobProgress, ProductMetadata, RunRecord } from "./types";

type ViewKey = "search" | "data" | "seo" | "manual" | "history" | "about";

const NAV: { key: ViewKey; label: string; icon: typeof Search; hint: string }[] = [
  { key: "search", label: "Пошук", icon: Search, hint: "Хто посилається на домен" },
  { key: "data", label: "Дані та індекс", icon: Database, hint: "Файли графа й індексація" },
  { key: "seo", label: "SEO discovery", icon: Sparkles, hint: "Згадки через пошукові API" },
  { key: "manual", label: "Ручний аудит", icon: FileSearch, hint: "Перевірка сторінок вручну" },
  { key: "history", label: "Історія", icon: History, hint: "Минулі операції" },
];

function App() {
  const store = useAppStore();
  const [view, setView] = useState<ViewKey>("search");

  const refreshIndex = useCallback(async (config: AppConfig) => {
    try {
      store.setIndex(await invoke<IndexStatus>("index_status", { config }));
    } catch (cause) {
      // A bad path must not blank the whole app — the Data view explains it.
      store.setIndex(null);
      store.setError(String(cause));
    }
  }, []);

  const refresh = useCallback(async () => {
    const [metadata, config, history] = await Promise.all([
      invoke<ProductMetadata>("product_metadata"),
      invoke<AppConfig>("load_default_config"),
      invoke<RunRecord[]>("get_run_history"),
    ]);
    store.setMetadata(metadata);
    store.setConfig(config);
    store.setHistory(history);
    await refreshIndex(config);
  }, [refreshIndex]);

  useEffect(() => {
    refresh().catch((cause) => store.setError(String(cause)));
    const unlisten = listen<JobProgress>("job-progress", ({ payload }) => store.setJob(payload));
    return () => void unlisten.then((stop) => stop());
  }, [refresh]);

  // First run with nothing indexed: start people on the page that explains it.
  useEffect(() => {
    if (store.index && !store.index.tiers.some((tier) => tier.state === "ready")) {
      setView("data");
    }
  }, [store.index !== null]);

  const reloadStatus = () => void refreshIndex(store.config);
  const reloadHistory = async () =>
    store.setHistory(await invoke<RunRecord[]>("get_run_history"));

  return (
    <div className="flex min-h-screen text-slate-100">
      <aside className="flex w-60 shrink-0 flex-col border-r border-white/10 bg-slate-950/70 backdrop-blur-xl">
        <div className="flex items-center gap-3 px-5 py-5">
          <div className="grid h-10 w-10 place-items-center rounded-xl bg-cyan-400 text-slate-950 shadow-lg shadow-cyan-400/20">
            <Radar size={22} />
          </div>
          <div className="min-w-0">
            <h1 className="text-sm font-bold tracking-tight">Web Radar</h1>
            <p className="truncate text-[11px] text-slate-500">
              Граф доменів Common Crawl
            </p>
          </div>
        </div>

        <nav className="flex-1 space-y-1 px-3">
          {NAV.map((item) => (
            <button
              key={item.key}
              className={view === item.key ? "nav-active" : "nav-button"}
              onClick={() => setView(item.key)}
              title={item.hint}
            >
              <item.icon size={16} className="shrink-0" />
              <span className="truncate">{item.label}</span>
              {item.key === "history" && store.history.length > 0 && (
                <span className="badge ml-auto">{store.history.length}</span>
              )}
              {item.key === "data" && store.index?.blockers.length ? (
                <TriangleAlert size={14} className="ml-auto text-amber-400" />
              ) : null}
            </button>
          ))}
        </nav>

        <div className="border-t border-white/[.06] p-3">
          <button
            className={view === "about" ? "nav-active" : "nav-button"}
            onClick={() => setView("about")}
          >
            <Info size={16} className="shrink-0" />
            <span>Про застосунок</span>
          </button>
          {store.metadata && (
            <button
              onClick={() => void openUrl(store.metadata!.authorUrl)}
              title={`Інші проєкти та послуги — ${store.metadata.author}`}
              className="author-link group"
            >
              <Sparkles className="h-3.5 w-3.5 shrink-0 text-indigo-400/70 group-hover:text-indigo-400" />
              <span className="truncate">More by {store.metadata.author}</span>
              <ExternalLink className="ml-auto h-3 w-3 shrink-0 opacity-0 transition group-hover:opacity-60" />
            </button>
          )}
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <UpdateBanner />
        {store.error && (
          <div className="flex items-start gap-3 border-b border-rose-400/20 bg-rose-500/10 px-7 py-3 text-sm text-rose-200">
            <TriangleAlert size={17} className="mt-0.5 shrink-0" />
            <p className="min-w-0 flex-1 whitespace-pre-wrap break-words">{store.error}</p>
            <button className="shrink-0 text-rose-300/70 hover:text-rose-200" onClick={() => store.setError(null)}>
              <X size={16} />
            </button>
          </div>
        )}
        {store.notice && (
          <div className="flex items-start gap-3 border-b border-emerald-400/20 bg-emerald-500/10 px-7 py-3 text-sm text-emerald-200">
            <p className="min-w-0 flex-1 break-words">{store.notice}</p>
            <button className="shrink-0 text-emerald-300/70 hover:text-emerald-200" onClick={() => store.setNotice(null)}>
              <X size={16} />
            </button>
          </div>
        )}

        <main className="min-w-0 flex-1 overflow-y-auto px-7 py-7">
          {!store.metadata ? (
            <div className="grid h-full place-items-center text-slate-500">
              <LoaderCircle className="animate-spin" size={28} />
            </div>
          ) : view === "search" ? (
            <SearchView onGoToData={() => setView("data")} />
          ) : view === "data" ? (
            <DataView onStatusChange={reloadStatus} onHistoryChange={reloadHistory} />
          ) : view === "seo" ? (
            <SeoDiscoveryView domains={store.config.targets} />
          ) : view === "manual" ? (
            <ManualResearchView domains={store.config.targets} />
          ) : view === "history" ? (
            <HistoryView history={store.history} onRefresh={reloadHistory} />
          ) : (
            <AboutView />
          )}
        </main>

        <JobBar />
      </div>
    </div>
  );
}

export default App;
