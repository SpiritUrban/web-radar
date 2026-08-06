import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { AlertTriangle, CheckCircle2, ClipboardPaste, ExternalLink, FileSearch, Link2, LoaderCircle, Search, ShieldAlert, XCircle } from "lucide-react";
import type { ManualAuditReport } from "../types";

const cleanDomain = (value: string) => value.replace(/^https?:\/\//, "").replace(/^www\./, "").replace(/\/.*$/, "");
const deriveBrand = (domain: string) => cleanDomain(domain).split(".")[0].split(/[-_]/).map((w) => w[0]?.toUpperCase() + w.slice(1)).join(" ");
const extractUrls = (text: string) => [...new Set((text.match(/https?:\/\/[^\s<>"'\])}]+/g) ?? []).map((url) => url.replace(/[.,;:]+$/, "")))];
const engines = {
  Google: (q: string) => `https://www.google.com/search?q=${encodeURIComponent(q)}`,
  Brave: (q: string) => `https://search.brave.com/search?q=${encodeURIComponent(q)}`,
  Bing: (q: string) => `https://www.bing.com/search?q=${encodeURIComponent(q)}`,
  DuckDuckGo: (q: string) => `https://duckduckgo.com/?q=${encodeURIComponent(q)}`,
};

export default function ManualResearchView({ domains }: { domains: string[] }) {
  const available = useMemo(() => [...new Set(domains.map(cleanDomain).filter(Boolean))], [domains]);
  const [domain, setDomain] = useState(available[0] ?? "");
  const [brand, setBrand] = useState(deriveBrand(available[0] ?? ""));
  const [input, setInput] = useState("");
  const [engine, setEngine] = useState<keyof typeof engines>("Google");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<ManualAuditReport | null>(null);
  const urls = useMemo(() => extractUrls(input), [input]);
  const ownUrls = useMemo(() => urls.filter((raw) => { try { const host = new URL(raw).hostname.replace(/^www\./, ""); return host === domain || host.endsWith(`.${domain}`); } catch { return false; } }), [urls, domain]);
  const externalUrls = useMemo(() => urls.filter((url) => !ownUrls.includes(url)), [urls, ownUrls]);
  const queries = useMemo(() => [
    `"${brand}" -site:${domain}`,
    `"${domain}" -site:${domain}`,
    `"https://${domain}" -site:${domain}`,
    `intext:"${domain}" -site:${domain}`,
    `link:${domain}`,
    `"${brand}" (review OR reviews OR відгуки OR скарги) -site:${domain}`,
  ], [domain, brand]);

  const changeDomain = (next: string) => { setDomain(next); setBrand(deriveBrand(next)); setReport(null); };
  const openSearch = async (query: string) => {
    try {
      await openUrl(engines[engine](query));
      setError(null);
    } catch (cause) {
      setError(`Не вдалося відкрити ${engine}: ${String(cause)}. Перезапустіть застосунок після оновлення permissions.`);
    }
  };
  const pasteFromClipboard = async () => {
    try {
      const clipboard = await readText();
      if (!clipboard.trim()) throw new Error("Буфер обміну порожній");
      setInput((current) => current ? `${current}\n${clipboard}` : clipboard);
      setError(null);
    } catch {
      setError("Не вдалося прочитати системний буфер. Скопіюйте адресу посилання у браузері та спробуйте ще раз.");
    }
  };
  const audit = async () => {
    setRunning(true); setError(null);
    try { setReport(await invoke<ManualAuditReport>("run_manual_audit", { request: { targetDomain: domain, brand, urls: externalUrls } })); }
    catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  };

  return <section>
    <p className="eyebrow"><FileSearch size={14}/> Ручне SERP-дослідження</p>
    <h2 className="mt-2 text-3xl font-semibold tracking-tight">Пошук без API-ключа</h2>
    <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-400">Усього три дії — застосунок проведе вас по черзі.</p>
    <div className="mt-6 grid gap-3 md:grid-cols-3">
      <Step number="1" title="Відкрийте пошук" detail="Натисніть кнопку біля одного або кількох готових запитів."/>
      <Step number="2" title="Скопіюйте адреси" detail="У результатах скопіюйте адреси релевантних зовнішніх сторінок."/>
      <Step number="3" title="Вставте й перевірте" detail="Вставте список нижче та запустіть фактичний аудит backlinks."/>
    </div>
    <div className="mt-6 flex gap-3 rounded-xl border border-amber-400/20 bg-amber-400/[.06] p-4 text-xs leading-5 text-amber-100/80"><ShieldAlert className="mt-0.5 shrink-0" size={17}/><span>Web Radar не скрейпить сторінки результатів Google і не обходить CAPTCHA. Перевіряються лише вибрані вами URL, послідовно, з robots.txt, паузою, timeout і лімітом 2 MB.</span></div>

    <div className="mt-6 grid gap-6 xl:grid-cols-[.85fr_1.15fr]">
      <div className="space-y-6">
        <div className="panel">
          <h3 className="text-sm font-semibold">1. Ціль</h3>
          <select className="input mt-4 w-full" value={domain} onChange={(e) => changeDomain(e.target.value)}>{available.map((item) => <option key={item}>{item}</option>)}</select>
          <label className="mt-4 block"><span className="mb-2 block text-xs text-slate-400">Назва бренду</span><input className="input w-full" value={brand} onChange={(e) => setBrand(e.target.value)}/></label>
        </div>
        <div className="panel">
          <h3 className="text-sm font-semibold">2. Відкрийте операторні запити</h3>
          <div className="mt-4 space-y-3">{queries.map((query, index) => <div key={query} className="rounded-xl border border-white/10 bg-slate-950/40 p-3"><p className="break-all font-mono text-[11px] leading-5 text-slate-400">{query}</p><button className="add-button mt-3 h-9 w-full justify-center" onClick={() => void openSearch(query)}><Search size={14}/>Відкрити в {engine}</button></div>)}</div>
        </div>
        <div className="panel">
          <h3 className="text-sm font-semibold">3. Вставте адреси знайдених сторінок</h3><p className="mt-2 text-xs leading-5 text-slate-500">У браузері виберіть результат з ІНШОГО домену, не з <b className="text-slate-300">{domain}</b>. Правою кнопкою на результаті → «Копіювати адресу посилання». По одній адресі в рядку. Можна вставити й увесь скопійований текст — URL витягнуться автоматично.</p><button className="secondary-button mt-4 w-full justify-center" onClick={pasteFromClipboard}><ClipboardPaste size={15}/>Вставити з буфера</button>
          <textarea className="input mt-4 h-44 w-full resize-y py-3" value={input} onChange={(e) => setInput(e.target.value)} placeholder={'https://example.org/page-about-your-brand\nhttps://another-site.com/article'}/>
          <div className="mt-3 space-y-2 text-xs"><div className="flex items-center justify-between text-slate-500"><span>Зовнішніх URL для перевірки: <b className="text-cyan-300">{externalUrls.length}</b></span><span>Максимум 50</span></div>{ownUrls.length > 0 && <div className="rounded-lg border border-amber-400/20 bg-amber-400/[.06] px-3 py-2 leading-5 text-amber-200">Відкинуто власних URL: {ownUrls.length}. Сторінка {domain} не є беклінком на саму себе — потрібна сторінка іншого сайту, яка посилається на {domain}.</div>}</div>
          <button className="run-button mt-5" disabled={!domain || !externalUrls.length || running} onClick={audit}>{running ? <LoaderCircle className="animate-spin" size={18}/> : <FileSearch size={18}/>} {running ? "Перевіряємо сторінки…" : `Проаналізувати ${externalUrls.length} зовнішніх сторінок`}</button>
        </div>
      </div>

      <div className="space-y-6">
        {error && <div className="flex gap-3 rounded-xl border border-rose-400/30 bg-rose-400/10 p-4 text-sm text-rose-200"><AlertTriangle size={18}/><span>{error}</span></div>}
        {!report ? <div className="panel grid min-h-[420px] place-items-center text-center"><div><Link2 className="mx-auto text-slate-700" size={42}/><h3 className="mt-4 font-semibold text-slate-300">Очікуємо список сторінок</h3><p className="mt-2 max-w-md text-sm leading-6 text-slate-500">Аудит відрізнить підтверджений backlink від простої текстової згадки.</p></div></div> : <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label="Перевірено" value={report.audited}/><Metric label="Backlinks" value={report.confirmedBacklinks}/><Metric label="Згадки без link" value={report.mentionsWithoutLink}/><Metric label="Пропущено" value={report.blockedOrSkipped}/></div>
          <div className="panel p-0 overflow-hidden"><div className="border-b border-white/10 px-5 py-4"><h3 className="text-sm font-semibold">Аудит сторінок</h3></div><div className="max-h-[700px] overflow-auto">{report.pages.map((page) => <article key={page.url} className="border-b border-white/[.06] p-5 last:border-0"><div className="flex items-start gap-3">{page.hasBacklink ? <CheckCircle2 className="mt-0.5 shrink-0 text-emerald-400" size={18}/> : page.status === "audited" ? <Search className="mt-0.5 shrink-0 text-amber-400" size={18}/> : <XCircle className="mt-0.5 shrink-0 text-rose-400" size={18}/>}<div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><span className={`rounded px-2 py-1 text-[10px] font-bold uppercase ${page.hasBacklink ? "bg-emerald-400/10 text-emerald-400" : "bg-white/5 text-slate-500"}`}>{page.hasBacklink ? `${page.backlinkCount} backlink` : page.status}</span>{page.httpStatus && <span className="text-[10px] text-slate-600">HTTP {page.httpStatus}</span>}{page.relValues.map((rel) => <span key={rel} className="rounded bg-amber-400/10 px-2 py-1 text-[10px] text-amber-300">rel={rel}</span>)}</div><h4 className="mt-2 text-sm font-semibold text-slate-200">{page.title || page.sourceDomain || "Сторінку не прочитано"}</h4><p className="mt-1 truncate text-xs text-slate-500">{page.url}</p>{page.anchors.length > 0 && <p className="mt-2 text-xs text-cyan-300">Anchor: {page.anchors.join(" · ")}</p>}<div className="mt-2 flex gap-3 text-[10px] text-slate-500"><span>Brand: {page.hasBrandMention ? "так" : "ні"}</span><span>Domain text: {page.hasDomainMention ? "так" : "ні"}</span></div>{page.status !== "audited" && <p className="mt-2 text-xs text-rose-300/80">{page.note}</p>}</div><button className="icon-button" onClick={() => openUrl(page.url)}><ExternalLink size={15}/></button></div></article>)}</div></div>
        </>}
      </div>
    </div>
  </section>;
}
function Step({number, title, detail}:{number:string; title:string; detail:string}) { return <div className="panel flex items-start gap-3 p-4"><span className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-cyan-400 text-xs font-bold text-slate-950">{number}</span><div><p className="text-sm font-semibold text-slate-200">{title}</p><p className="mt-1 text-xs leading-5 text-slate-500">{detail}</p></div></div>; }
function Metric({label, value}:{label:string; value:number}) { return <div className="panel p-4"><p className="text-2xl font-bold text-cyan-300">{value}</p><p className="mt-1 text-[11px] text-slate-500">{label}</p></div>; }
