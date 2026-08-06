import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowDownUp, ArrowLeftRight, Clock, Database, Download, ExternalLink, Gauge,
  Link2, LoaderCircle, Radar, Search, Sparkles, Target, TriangleAlert,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useAppStore } from "../store";
import { cleanDomain, formatCount, formatRank, metricLabel } from "../format";
import type { LinkEntry, QueryOutcome } from "../types";

type Direction = "inbound" | "outbound";
type SortKey = "rank" | "domain";

const PAGE = 100;

export default function SearchView({ onGoToData }: { onGoToData: () => void }) {
  const store = useAppStore();
  const [input, setInput] = useState("");
  const [recent, setRecent] = useState<string[]>([]);
  const [direction, setDirection] = useState<Direction>("inbound");
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<SortKey>("rank");
  const [shown, setShown] = useState(PAGE);

  const busy = store.busy === "query";
  const report = store.result?.report ?? null;
  const capabilities = store.result?.capabilities ?? null;

  const search = async (raw: string) => {
    const domain = cleanDomain(raw);
    if (!domain || busy) return;
    store.setBusy("query");
    store.setError(null);
    setFilter("");
    setShown(PAGE);
    try {
      const outcome = await invoke<QueryOutcome>("query_domain", {
        config: store.config,
        domain,
      });
      store.setResult(outcome);
      setDirection(outcome.report.inbound.length || !outcome.report.outbound.length ? "inbound" : "outbound");
      setRecent((previous) => [domain, ...previous.filter((item) => item !== domain)].slice(0, 6));
    } catch (cause) {
      store.setResult(null);
      store.setError(String(cause));
    } finally {
      store.setBusy(null);
    }
  };

  const exportReport = async (format: "json" | "csv") => {
    if (!report) return;
    try {
      const path = await invoke<string>("export_report", {
        config: store.config,
        domain: report.domain,
        format,
      });
      store.setNotice(`Збережено: ${path}`);
    } catch (cause) {
      store.setError(String(cause));
    }
  };

  const entries: LinkEntry[] = report ? report[direction] : [];
  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const list = needle ? entries.filter((entry) => entry.domain.includes(needle)) : entries;
    if (sort === "domain") {
      return [...list].sort((a, b) => a.domain.localeCompare(b.domain));
    }
    return list;
  }, [entries, filter, sort]);

  const total = report ? (direction === "inbound" ? report.inboundTotal : report.outboundTotal) : 0;

  return (
    <section>
      <p className="eyebrow"><Radar size={14} /> Запит до графа</p>
      <h2 className="mt-2 text-3xl font-semibold tracking-tight">Хто посилається на домен</h2>
      <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-400">
        Введіть домен — застосунок відповість із локального індексу за мілісекунди, не читаючи
        десятки гігабайтів графа.
      </p>

      <div className="mt-6 flex flex-wrap gap-2">
        <div className="relative min-w-[320px] flex-1">
          <Search size={17} className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            className="input h-12 w-full pl-11 text-base"
            value={input}
            autoFocus
            placeholder="example.com або https://www.example.com/page"
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && void search(input)}
          />
        </div>
        <button className="run-button h-12 w-auto px-7" disabled={busy || !input.trim()} onClick={() => void search(input)}>
          {busy ? <LoaderCircle className="animate-spin" size={18} /> : <Search size={18} />}
          {busy ? "Шукаємо…" : "Знайти"}
        </button>
      </div>

      {(store.config.targets.length > 0 || recent.length > 0) && (
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-slate-500">
          {recent.length > 0 && <span>Нещодавні:</span>}
          {recent.map((domain) => (
            <button key={domain} className="chip-button" onClick={() => { setInput(domain); void search(domain); }}>
              <Clock size={12} />{domain}
            </button>
          ))}
          {store.config.targets.length > 0 && <span className="ml-2">Із config.toml:</span>}
          {store.config.targets.slice(0, 6).map((domain) => (
            <button key={domain} className="chip-button" onClick={() => { setInput(domain); void search(domain); }}>
              <Target size={12} />{cleanDomain(domain)}
            </button>
          ))}
        </div>
      )}

      {!report ? (
        <EmptyState nodeCount={store.index?.nodeCount ?? 0} onGoToData={onGoToData} />
      ) : (
        <div className="mt-7 space-y-5">
          <div className="grid gap-3 md:grid-cols-4">
            <StatCard
              icon={<Gauge size={16} />}
              label={metricLabel(report.metric)}
              value={formatRank(report.rank)}
              hint={report.position ? `#${formatCount(report.position)} у світі` : capabilities?.ranks ? "немає у файлі ranks" : "рівень не побудований"}
            />
            <StatCard icon={<Link2 size={16} />} label="Вхідних доменів" value={formatCount(report.inboundTotal)} hint="хто посилається сюди" />
            <StatCard icon={<ArrowLeftRight size={16} />} label="Вихідних доменів" value={formatCount(report.outboundTotal)} hint="куди посилається" />
            <StatCard icon={<Clock size={16} />} label="Час відповіді" value={`${formatCount(report.elapsedMs)} мс`} hint={report.found ? `id ${formatCount(report.nodeId ?? 0)}` : "домен не знайдено"} />
          </div>

          {report.warnings.map((warning) => (
            <div key={warning} className="flex items-start gap-3 rounded-xl border border-amber-400/20 bg-amber-400/[.06] p-4 text-sm text-amber-100/90">
              <TriangleAlert size={17} className="mt-0.5 shrink-0" />
              <p className="min-w-0 flex-1">{warning}</p>
              {warning.includes("Рівень") && (
                <button className="secondary-button shrink-0" onClick={onGoToData}>
                  <Database size={14} /> До індексу
                </button>
              )}
            </div>
          ))}

          {report.found && (
            <div className="panel p-0">
              <div className="flex flex-wrap items-center gap-3 border-b border-white/10 px-5 py-4">
                <div className="flex rounded-lg border border-white/10 bg-white/[.03] p-1 text-xs">
                  {(["inbound", "outbound"] as const).map((key) => (
                    <button
                      key={key}
                      className={direction === key ? "tab-active" : "tab-button"}
                      onClick={() => { setDirection(key); setShown(PAGE); }}
                    >
                      {key === "inbound" ? "Вхідні" : "Вихідні"}
                      <span className="badge">{formatCount(key === "inbound" ? report.inboundTotal : report.outboundTotal)}</span>
                    </button>
                  ))}
                </div>
                <input
                  className="input h-9 max-w-[240px] text-xs"
                  placeholder="Фільтр за доменом"
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                />
                <button
                  className="secondary-button"
                  onClick={() => setSort(sort === "rank" ? "domain" : "rank")}
                  title="Змінити порядок"
                >
                  <ArrowDownUp size={14} /> {sort === "rank" ? "За рейтингом" : "За абеткою"}
                </button>
                <div className="ml-auto flex gap-2">
                  <button className="secondary-button" onClick={() => void exportReport("json")}>
                    <Download size={14} /> JSON
                  </button>
                  <button className="secondary-button" onClick={() => void exportReport("csv")}>
                    <Download size={14} /> CSV
                  </button>
                </div>
              </div>

              {visible.length === 0 ? (
                <p className="p-10 text-center text-sm text-slate-500">
                  {entries.length === 0
                    ? direction === "inbound"
                      ? "Жоден домен у цьому випуску графа не посилається сюди."
                      : "Цей домен нікуди не посилається у цьому випуску графа."
                    : "Нічого не знайдено за фільтром."}
                </p>
              ) : (
                <>
                  <div className="grid grid-cols-[46px_1fr_150px_120px] gap-3 border-b border-white/[.06] px-5 py-2.5 text-[10px] font-semibold uppercase tracking-wider text-slate-500">
                    <span>#</span><span>Домен</span>
                    <span className="text-right">{metricLabel(report.metric)}</span>
                    <span className="text-right">Позиція</span>
                  </div>
                  {visible.slice(0, shown).map((entry, index) => (
                    <div key={entry.domain} className="grid grid-cols-[46px_1fr_150px_120px] items-center gap-3 border-b border-white/[.04] px-5 py-2.5 text-sm last:border-0 hover:bg-white/[.02]">
                      <span className="font-mono text-xs text-slate-600">{index + 1}</span>
                      <button
                        className="group flex min-w-0 items-center gap-2 text-left text-slate-200 hover:text-cyan-300"
                        onClick={() => void openUrl(`https://${entry.domain}`)}
                        title={`Відкрити https://${entry.domain}`}
                      >
                        <span className="truncate">{entry.domain}</span>
                        <ExternalLink size={12} className="shrink-0 opacity-0 transition group-hover:opacity-60" />
                      </button>
                      <span className="text-right font-mono text-xs text-slate-400">{formatRank(entry.rank)}</span>
                      <span className="text-right font-mono text-xs text-slate-500">
                        {entry.position ? `#${formatCount(entry.position)}` : "—"}
                      </span>
                    </div>
                  ))}
                  <div className="flex items-center justify-between px-5 py-3 text-xs text-slate-500">
                    <span>
                      Показано {formatCount(Math.min(shown, visible.length))} із {formatCount(visible.length)}
                      {total > entries.length ? ` (у графі ${formatCount(total)})` : ""}
                    </span>
                    {shown < visible.length && (
                      <button className="secondary-button" onClick={() => setShown(shown + 500)}>
                        Показати ще 500
                      </button>
                    )}
                  </div>
                </>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function StatCard({ icon, label, value, hint }: { icon: React.ReactNode; label: string; value: string; hint: string }) {
  return (
    <div className="panel p-4">
      <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-500">
        <span className="text-cyan-400">{icon}</span>{label}
      </div>
      <p className="mt-2 truncate font-mono text-xl font-semibold text-slate-100" title={value}>{value}</p>
      <p className="mt-1 truncate text-[11px] text-slate-500" title={hint}>{hint}</p>
    </div>
  );
}

function EmptyState({ nodeCount, onGoToData }: { nodeCount: number; onGoToData: () => void }) {
  const store = useAppStore();
  return (
    <div className="panel mt-7 grid min-h-[380px] place-items-center text-center">
      <div className="max-w-lg">
        <Radar className="mx-auto text-slate-700" size={46} />
        <h3 className="mt-4 font-semibold text-slate-300">
          {nodeCount > 0 ? `${formatCount(nodeCount)} доменів готові до запитів` : "Індекс ще не побудований"}
        </h3>
        <p className="mt-2 text-sm leading-6 text-slate-500">
          {nodeCount > 0
            ? "Введіть будь-який домен вище. Відповідь приходить із локального індексу — інтернет не потрібен."
            : "Спершу вкажіть файли графа Common Crawl і побудуйте індекс — після цього запити виконуються миттєво."}
        </p>
        {nodeCount === 0 && (
          <button className="secondary-button mx-auto mt-5" onClick={onGoToData}>
            <Database size={15} /> Перейти до даних та індексу
          </button>
        )}
        {store.metadata && (
          <p className="mt-8 text-[11px] text-slate-600">
            <Sparkles size={11} className="mr-1 inline text-indigo-400/60" />
            Web Radar {store.metadata.version} — {store.metadata.author}
          </p>
        )}
      </div>
    </div>
  );
}
