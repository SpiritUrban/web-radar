import { Check, Database, History, Play, RefreshCw, Square, TriangleAlert } from "lucide-react";
import { formatDateTime, metricLabel } from "../format";
import type { RunRecord } from "../types";

const STATUS_LABELS: Record<string, string> = {
  running: "виконується",
  completed: "завершено",
  failed: "помилка",
  cancelled: "скасовано",
};

export default function HistoryView({
  history,
  onRefresh,
}: {
  history: RunRecord[];
  onRefresh: () => Promise<void>;
}) {
  return (
    <section>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <p className="eyebrow"><History size={14} /> Локальна база</p>
          <h2 className="mt-2 text-3xl font-semibold tracking-tight">Історія операцій</h2>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-slate-400">
            Індексації та повні сканування зберігаються у локальному SQLite поруч із застосунком.
            Швидкі запити сюди не потрапляють — вони не змінюють дані.
          </p>
        </div>
        <button className="secondary-button" onClick={() => void onRefresh()}>
          <RefreshCw size={15} /> Оновити
        </button>
      </div>

      <div className="panel mt-7 overflow-hidden p-0">
        <div className="grid grid-cols-[64px_130px_1fr_150px_120px] gap-4 border-b border-white/10 px-6 py-3 text-[10px] font-semibold uppercase tracking-wider text-slate-500">
          <span>ID</span><span>Тип</span><span>Запуск</span><span>Метрика</span><span>Статус</span>
        </div>
        {history.length === 0 ? (
          <div className="p-12 text-center text-sm text-slate-500">
            Історія з'явиться після першої індексації або сканування.
          </div>
        ) : (
          history.map((run) => (
            <div
              key={run.id}
              className="grid grid-cols-[64px_130px_1fr_150px_120px] items-center gap-4 border-b border-white/[.06] px-6 py-3.5 text-sm last:border-0"
            >
              <span className="font-mono text-xs text-slate-600">#{run.id}</span>
              <span className="flex items-center gap-2 text-xs text-slate-400">
                {run.kind === "index" ? <Database size={13} /> : <Play size={13} />}
                {run.kind === "index" ? "індексація" : "сканування"}
              </span>
              <div className="min-w-0">
                <p>{formatDateTime(run.startedAt)}</p>
                {run.targets.length > 0 && (
                  <p className="mt-0.5 truncate text-xs text-slate-500">{run.targets.join(", ")}</p>
                )}
                {run.error && (
                  <p className="mt-1 flex items-start gap-1.5 text-xs text-rose-400">
                    <TriangleAlert size={12} className="mt-0.5 shrink-0" />
                    <span className="line-clamp-2">{run.error}</span>
                  </p>
                )}
              </div>
              <span className="text-xs text-slate-400">{metricLabel(run.rankMetric)}</span>
              <span className={`status status-${run.status}`}>
                {run.status === "completed" && <Check size={12} />}
                {run.status === "cancelled" && <Square size={10} />}
                {STATUS_LABELS[run.status] ?? run.status}
              </span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
