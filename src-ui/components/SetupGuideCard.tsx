import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ArrowDownToLine, Check, Copy, ExternalLink, FolderOpen, PackageOpen, RefreshCw,
} from "lucide-react";
import { useState } from "react";
import { formatBytes } from "../format";
import type { SetupGuide, SourceStatus } from "../types";

/**
 * The whole "how do I get the data" answer, in one place.
 *
 * Web Radar does not download the graph itself — it is ~16 GB from a third
 * party. That makes this card the product's real first-run experience, so it
 * carries everything: exact file names, direct links, both sizes, which files
 * must be unpacked, and where they go.
 */
export default function SetupGuideCard({
  setup,
  sources,
  targetDir,
  indexBytes,
  onChooseFolder,
}: {
  setup: SetupGuide;
  sources: SourceStatus[];
  targetDir: string;
  /** Estimated size of the full index, so the disk budget is stated once. */
  indexBytes: number;
  onChooseFolder: () => void;
}) {
  const [copied, setCopied] = useState<string | null>(null);

  const statusOf = (kind: string) => sources.find((source) => source.kind === kind);
  const missing = setup.files.filter((file) => !statusOf(file.kind)?.exists);
  const needsUnpacking = sources.filter(
    (source) => source.exists && source.compressed && source.mustBeUnpacked,
  );
  if (missing.length === 0 && needsUnpacking.length === 0) return null;

  const copy = async (text: string, key: string) => {
    try {
      await writeText(text);
      setCopied(key);
      setTimeout(() => setCopied((current) => (current === key ? null : current)), 1600);
    } catch {
      setCopied(null);
    }
  };

  const remaining = missing.reduce((sum, file) => sum + file.downloadBytes, 0);

  return (
    <section className="panel border-cyan-400/25 bg-cyan-400/[.04]">
      <div className="panel-title">
        <div className="icon-box"><ArrowDownToLine size={18} /></div>
        <div>
          <h3>Як отримати дані графа</h3>
          <p>
            Випуск {setup.crawl} · залишилось завантажити ≈{formatBytes(remaining)}
          </p>
        </div>
      </div>

      <p className="mt-4 text-xs leading-5 text-slate-400">
        Web Radar не качає граф сам: це {formatBytes(setup.totalDownloadBytes)} зі стороннього
        сервера, і мовчазне багатогодинне завантаження всередині застосунку було б гірше за
        чесну інструкцію. Тому — три кроки.
      </p>

      <ol className="mt-5 space-y-4">
        <li>
          <p className="text-xs font-semibold uppercase tracking-wider text-cyan-300">
            1. Завантажте файли
          </p>
          <div className="mt-3 space-y-2">
            {setup.files.map((file) => {
              const status = statusOf(file.kind);
              const present = status?.exists ?? false;
              const stillPacked = present && status!.compressed && status!.mustBeUnpacked;
              return (
                <div
                  key={file.kind}
                  className={`flex flex-wrap items-center gap-3 rounded-xl border p-3 ${
                    present && !stillPacked
                      ? "border-emerald-400/20 bg-emerald-400/[.05]"
                      : "border-white/10 bg-slate-950/40"
                  }`}
                >
                  <span className="w-5 shrink-0">
                    {present && !stillPacked ? (
                      <Check size={16} className="text-emerald-400" />
                    ) : stillPacked ? (
                      <PackageOpen size={16} className="text-amber-400" />
                    ) : (
                      <ArrowDownToLine size={16} className="text-slate-500" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-mono text-xs text-slate-200" title={file.archiveName}>
                      {file.archiveName}
                    </p>
                    <p className="mt-0.5 text-[11px] text-slate-500">
                      {formatBytes(file.downloadBytes)} завантаження ·{" "}
                      {file.mustBeUnpacked
                        ? `${formatBytes(file.unpackedBytes)} після розпакування`
                        : "розпаковувати не треба"}{" "}
                      · {file.purpose}
                    </p>
                    {stillPacked && (
                      <p className="mt-1 text-[11px] text-amber-300">
                        Файл на місці, але ще стиснутий — розпакуйте його.
                      </p>
                    )}
                  </div>
                  {!present && (
                    <>
                      <button className="secondary-button shrink-0" onClick={() => void openUrl(file.url)}>
                        <ExternalLink size={13} /> Завантажити
                      </button>
                      <button
                        className="icon-button h-9 w-9 shrink-0"
                        title="Скопіювати посилання"
                        onClick={() => void copy(file.url, file.kind)}
                      >
                        {copied === file.kind ? <Check size={14} className="text-emerald-400" /> : <Copy size={14} />}
                      </button>
                    </>
                  )}
                </div>
              );
            })}
          </div>
        </li>

        <li>
          <p className="text-xs font-semibold uppercase tracking-wider text-cyan-300">
            2. Розпакуйте vertices і edges
          </p>
          <p className="mt-2 text-xs leading-5 text-slate-400">
            Запити читають ці два файли за зміщенням, а gzip такого не дає. Файл{" "}
            <b className="text-slate-300">ranks</b> розпаковувати не треба — він читається один
            раз послідовно, тож нехай лишається .gz і економить{" "}
            {formatBytes(
              (setup.files.find((f) => f.kind === "ranks")?.unpackedBytes ?? 0) -
                (setup.files.find((f) => f.kind === "ranks")?.downloadBytes ?? 0),
            )}{" "}
            диска.
          </p>
          <div className="mt-2 rounded-lg border border-white/10 bg-slate-950/60 p-3">
            <code className="block break-all font-mono text-[11px] text-slate-400">
              tar -xzf {setup.files[1]?.archiveName ?? "*.txt.gz"}
            </code>
            <p className="mt-1 text-[10px] text-slate-600">
              У Windows 10/11 команда tar вбудована. Підійде і 7-Zip.
            </p>
          </div>
        </li>

        <li>
          <p className="text-xs font-semibold uppercase tracking-wider text-cyan-300">
            3. Покладіть їх сюди
          </p>
          <div className="mt-2 flex gap-2">
            <code className="input flex items-center break-all font-mono text-[11px] text-slate-300">
              {targetDir || "—"}
            </code>
            <button
              className="icon-button"
              title="Скопіювати шлях"
              onClick={() => void copy(targetDir, "dir")}
            >
              {copied === "dir" ? <Check size={15} className="text-emerald-400" /> : <Copy size={15} />}
            </button>
            <button className="browse-button" title="Обрати іншу теку" onClick={onChooseFolder}>
              <FolderOpen size={17} />
            </button>
          </div>
          <p className="mt-2 text-[11px] text-slate-500">
            Після цього натисніть «Побудувати все». Файли займуть
            ≈{formatBytes(setup.totalResidentBytes)}, індекс — ще ≈{formatBytes(indexBytes)}.
          </p>
        </li>
      </ol>

      <div className="mt-5 flex items-start gap-2 border-t border-white/[.06] pt-4 text-[11px] leading-5 text-slate-500">
        <RefreshCw size={13} className="mt-0.5 shrink-0" />
        <p>
          Common Crawl публікує новий граф раз на 2–3 місяці. Щоб оновитися — завантажте
          свіжий випуск, замініть файли, і застосунок сам позначить рівні індексу як
          застарілі та запропонує перебудувати.{" "}
          <button className="text-cyan-400 hover:underline" onClick={() => void openUrl(setup.crawlListUrl)}>
            Перелік випусків
          </button>
        </p>
      </div>
    </section>
  );
}
