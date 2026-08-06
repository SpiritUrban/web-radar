import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle, Square } from "lucide-react";
import { useAppStore } from "../store";
import { formatBytes, formatDuration } from "../format";

/** Human labels for the stages the core reports. */
const STAGE_LABELS: Record<string, string> = {
  vertices_index: "Індексуємо домени",
  edges_index: "Розмічаємо зв'язки",
  ranks_partition: "Читаємо рейтинги",
  ranks_merge: "Зіставляємо рейтинги з доменами",
  inbound_partition: "Читаємо зв'язки",
  inbound_merge: "Будуємо індекс зворотних посилань",
  vertices_targets: "Шукаємо цільові домени",
  edges: "Скануємо зв'язки",
  vertices_neighbors: "Визначаємо назви сусідів",
  ranks: "Завантажуємо рейтинги",
  writing: "Записуємо результати",
};

/**
 * Bottom status bar for the one long job that can be running.
 *
 * Long operations are the app's main risk of feeling broken, so this shows
 * measured throughput and an ETA rather than an indeterminate spinner.
 */
export default function JobBar() {
  const store = useAppStore();
  const busy = store.busy === "index" || store.busy === "scan";
  if (!busy && !store.job) return null;

  const job = store.job;
  const percent = Math.round((job?.overall ?? 0) * 100);
  const label = job ? STAGE_LABELS[job.stage] ?? job.detail ?? job.stage : "Готуємо…";

  return (
    <div className="border-t border-white/10 bg-slate-950/90 px-7 py-3 backdrop-blur-xl">
      <div className="flex items-center gap-4">
        <LoaderCircle className="shrink-0 animate-spin text-cyan-400" size={17} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-4 text-xs">
            <span className="truncate text-slate-200">
              {store.busy === "scan" ? "Повне сканування · " : "Індексація · "}
              {label}
            </span>
            <span className="shrink-0 font-mono text-slate-500">
              {job && job.bytesPerSec > 0 && `${formatBytes(job.bytesPerSec)}/с · `}
              {job && job.etaSecs > 0 && `залишилось ~${formatDuration(job.etaSecs)} · `}
              {percent}%
            </span>
          </div>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-cyan-400 transition-all duration-200"
              style={{ width: `${Math.max(1.5, percent)}%` }}
            />
          </div>
          {job && job.stageTotal > 0 && (
            <p className="mt-1 text-[10px] text-slate-600">
              {formatBytes(job.stageDone)} із {formatBytes(job.stageTotal)} на цьому етапі ·
              минуло {formatDuration(job.elapsedSecs)}
            </p>
          )}
        </div>
        <button
          className="secondary-button shrink-0"
          onClick={() => void invoke("cancel_job")}
          title="Зупинити операцію на найближчій контрольній точці"
        >
          <Square size={13} /> Зупинити
        </button>
      </div>
    </div>
  );
}
