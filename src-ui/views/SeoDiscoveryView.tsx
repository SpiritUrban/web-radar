import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, ExternalLink, KeyRound, Link2, LoaderCircle, Search, ShieldCheck, Sparkles } from "lucide-react";
import type { SeoDiscoveryReport } from "../types";

const deriveBrand = (domain: string) => domain.replace(/^https?:\/\//, "").replace(/^www\./, "").split(".")[0].split(/[-_]/).filter(Boolean).map((word) => word[0]?.toUpperCase() + word.slice(1)).join(" ");

export default function SeoDiscoveryView({ domains }: { domains: string[] }) {
  const cleanDomains = useMemo(() => [...new Set(domains.map((d) => d.replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/^www\./, "")).filter(Boolean))], [domains]);
  const [provider, setProvider] = useState<"brave" | "google">("brave");
  const [apiKey, setApiKey] = useState("");
  const [googleCx, setGoogleCx] = useState("");
  const [brands, setBrands] = useState<Record<string, string>>({});
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<SeoDiscoveryReport | null>(null);

  const run = async () => {
    setRunning(true); setError(null);
    try {
      const result = await invoke<SeoDiscoveryReport>("run_seo_discovery", { request: {
        provider, apiKey, googleCx, country: "UA", resultsPerQuery: 10,
        targets: cleanDomains.map((domain) => ({ domain, brand: brands[domain] ?? deriveBrand(domain) })),
      }});
      setReport(result);
    } catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  };

  return <section>
    <p className="eyebrow"><Search size={14}/> SEO discovery</p>
    <h2 className="mt-2 text-3xl font-semibold tracking-tight">Згадки та потенційні беклінки</h2>
    <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-400">Операторні запити через структуровані API. Результат пошуку є доказом згадки, але не підтвердженням активного HTML-посилання.</p>

    <div className="mt-8 grid gap-6 xl:grid-cols-[.75fr_1.25fr]">
      <div className="space-y-6">
        <div className="panel">
          <div className="panel-title"><div className="icon-box"><KeyRound size={18}/></div><div><h3>Пошуковий провайдер</h3><p>Ключ використовується лише для поточного запиту</p></div></div>
          <div className="mt-5 grid grid-cols-2 gap-2">
            {(["brave", "google"] as const).map((item) => <button key={item} className={`metric ${provider === item ? "metric-active" : ""}`} onClick={() => setProvider(item)}><span className="metric-dot"/>{item === "brave" ? "Brave Search" : "Google PSE"}</button>)}
          </div>
          <label className="mt-5 block"><span className="mb-2 block text-xs font-semibold uppercase tracking-wider text-slate-400">API key</span><input className="input w-full" type="password" value={apiKey} onChange={(e) => setApiKey(e.target.value)} placeholder={provider === "brave" ? "Brave subscription token" : "Google API key"}/></label>
          {provider === "google" && <label className="mt-4 block"><span className="mb-2 block text-xs font-semibold uppercase tracking-wider text-slate-400">Search Engine ID (cx)</span><input className="input w-full" value={googleCx} onChange={(e) => setGoogleCx(e.target.value)} placeholder="Programmable Search Engine ID"/></label>}
          <div className="mt-4 flex gap-2 rounded-lg border border-emerald-400/15 bg-emerald-400/[.06] p-3 text-xs leading-5 text-emerald-200/80"><ShieldCheck className="mt-0.5 shrink-0" size={15}/>API key не записується в SQLite і не потрапляє до SEO-звіту.</div>
        </div>

        <div className="panel">
          <div className="panel-title"><div className="icon-box"><Sparkles size={18}/></div><div><h3>Цілі та бренди</h3><p>{cleanDomains.length} доменів із основного списку</p></div></div>
          <div className="mt-5 space-y-4">{cleanDomains.map((domain) => <label key={domain} className="block"><span className="mb-1 block text-xs text-cyan-300">{domain}</span><input className="input w-full" value={brands[domain] ?? deriveBrand(domain)} onChange={(e) => setBrands({...brands, [domain]: e.target.value})} placeholder="Назва бренду"/></label>)}</div>
          {!cleanDomains.length && <p className="mt-5 text-sm text-slate-500">Спочатку додайте домени на вкладці «Аналіз».</p>}
          <button className="run-button mt-6" disabled={running || !apiKey || !cleanDomains.length || (provider === "google" && !googleCx)} onClick={run}>{running ? <LoaderCircle className="animate-spin" size={18}/> : <Search size={18}/>} {running ? "Виконуємо операторні запити…" : "Зібрати SEO-дані"}</button>
          <p className="mt-3 text-center text-[11px] text-slate-500">6 запитів на домен · до 10 результатів із кожного</p>
        </div>
      </div>

      <div className="space-y-6">
        {error && <div className="flex gap-3 rounded-xl border border-rose-400/30 bg-rose-400/10 p-4 text-sm text-rose-200"><AlertTriangle className="shrink-0" size={18}/><span className="whitespace-pre-wrap">{error}</span></div>}
        {!report ? <div className="panel grid min-h-[380px] place-items-center text-center"><div><Link2 className="mx-auto text-slate-700" size={42}/><h3 className="mt-4 font-semibold text-slate-300">Ще немає SEO-звіту</h3><p className="mt-2 max-w-md text-sm leading-6 text-slate-500">Після запуску тут з’являться зовнішні джерела, тип доказу, snippets та обережні висновки.</p></div></div> : <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Metric label="Потенційні беклінки" value={report.potentialBacklinks}/><Metric label="Brand mentions" value={report.brandMentions}/><Metric label="Reputation" value={report.reputationMentions}/><Metric label="Унікальні домени" value={report.uniqueSourceDomains}/>
          </div>
          <div className="panel"><h3 className="text-sm font-semibold">Висновки</h3><div className="mt-4 space-y-3">{report.insights.map((insight, index) => <div key={index} className={`rounded-xl border p-4 text-sm ${insight.level === "warning" ? "border-amber-400/20 bg-amber-400/[.06]" : "border-white/10 bg-white/[.025]"}`}><p className="font-semibold text-slate-200">{insight.title}</p><p className="mt-1 leading-5 text-slate-400">{insight.detail}</p></div>)}</div></div>
          <div className="panel p-0 overflow-hidden"><div className="border-b border-white/10 px-5 py-4"><h3 className="text-sm font-semibold">Знайдені докази</h3><p className="mt-1 text-xs text-slate-500">{report.evidence.length} дедуплікованих сторінок</p></div><div className="max-h-[560px] overflow-auto">{report.evidence.map((item, index) => <article key={`${item.url}-${index}`} className="border-b border-white/[.06] p-5 last:border-0"><div className="flex items-start justify-between gap-4"><div className="min-w-0"><div className="mb-2 flex flex-wrap gap-2"><span className="rounded bg-cyan-400/10 px-2 py-1 text-[10px] font-bold uppercase text-cyan-300">{item.evidenceType.replaceAll("_", " ")}</span><span className="rounded bg-white/5 px-2 py-1 text-[10px] text-slate-500">{item.sourceDomain}</span></div><h4 className="text-sm font-semibold text-slate-200">{item.title}</h4><p className="mt-2 line-clamp-3 text-xs leading-5 text-slate-500">{item.snippet}</p><p className="mt-2 truncate font-mono text-[10px] text-slate-600">{item.query}</p></div><button className="icon-button" onClick={() => openUrl(item.url)}><ExternalLink size={15}/></button></div></article>)}</div></div>
        </>}
      </div>
    </div>
  </section>;
}

function Metric({label, value}:{label:string; value:number}) { return <div className="panel p-4"><p className="text-2xl font-bold text-cyan-300">{value}</p><p className="mt-1 text-[11px] text-slate-500">{label}</p></div>; }
