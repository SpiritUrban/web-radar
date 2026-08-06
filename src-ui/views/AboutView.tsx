import { openUrl } from "@tauri-apps/plugin-opener";
import { BookOpen, ExternalLink, Github, Globe, Radar, Sparkles } from "lucide-react";
import { useAppStore } from "../store";
import { formatBytes, formatCount } from "../format";

export default function AboutView() {
  const store = useAppStore();
  const meta = store.metadata;
  if (!meta) return null;

  return (
    <section className="mx-auto max-w-3xl">
      <p className="eyebrow"><Radar size={14} /> Про застосунок</p>
      <h2 className="mt-2 text-3xl font-semibold tracking-tight">{meta.productName}</h2>
      <p className="mt-2 text-sm leading-6 text-slate-400">
        Локальний інструмент для роботи з графом доменів Common Crawl: хто посилається на домен,
        куди він посилається і яку вагу мають ці зв'язки. Дані не залишають комп'ютер.
      </p>

      <div className="panel mt-7">
        <div className="grid gap-4 sm:grid-cols-2">
          <Fact label="Версія" value={meta.version} />
          <Fact label="Ліцензія" value="MIT" />
          <Fact
            label="Доменів у графі"
            value={store.index?.nodeCount ? formatCount(store.index.nodeCount) : "індекс не побудований"}
          />
          <Fact
            label="Розмір індексу"
            value={store.index?.totalBytes ? formatBytes(store.index.totalBytes) : "—"}
          />
        </div>
      </div>

      <div className="panel mt-5">
        <h3 className="text-sm font-semibold text-slate-200">Автор</h3>
        <p className="mt-2 text-sm leading-6 text-slate-400">
          {meta.productName} створив <b className="text-slate-200">{meta.author}</b> — інженер, який
          робить інструменти для роботи з великими даними та вебом. Інші проєкти й послуги — на
          персональному сайті.
        </p>
        <div className="mt-4 flex flex-wrap gap-2">
          <LinkButton icon={<Sparkles size={14} />} label="Проєкти та послуги автора" url={meta.authorUrl} />
          <LinkButton icon={<Github size={14} />} label="GitHub автора" url={meta.authorGithubUrl} />
          <LinkButton icon={<Globe size={14} />} label="Сайт Web Radar" url={meta.siteUrl} />
          <LinkButton icon={<BookOpen size={14} />} label="Джерело даних" url={meta.dataSourceUrl} />
        </div>
        <p className="mt-5 text-[11px] text-slate-600">{meta.copyright} · MIT License</p>
      </div>
    </section>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[11px] uppercase tracking-wider text-slate-500">{label}</p>
      <p className="mt-1 font-mono text-sm text-slate-200">{value}</p>
    </div>
  );
}

function LinkButton({ icon, label, url }: { icon: React.ReactNode; label: string; url: string }) {
  return (
    <button className="secondary-button" onClick={() => void openUrl(url)}>
      {icon}
      {label}
      <ExternalLink size={12} className="opacity-50" />
    </button>
  );
}
