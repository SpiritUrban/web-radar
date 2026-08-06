import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Download, LoaderCircle, X } from "lucide-react";

/**
 * Rule 28 of STAGE2_BRIEF: the updater plugin only matters if something calls
 * `check()`. Without this component an installed copy never learns that a newer
 * release exists.
 */
export default function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // A dev build has no update endpoint; failing quietly is correct here.
    check()
      .then((found) => found && setUpdate(found))
      .catch((cause) => console.info("update check skipped:", cause));
  }, []);

  if (!update || dismissed) return null;

  const install = async () => {
    setInstalling(true);
    setError(null);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (cause) {
      setError(String(cause));
      setInstalling(false);
    }
  };

  return (
    <div className="flex items-center gap-3 border-b border-cyan-400/20 bg-cyan-400/[.08] px-7 py-2.5 text-sm text-cyan-100">
      <span className="text-base">🎉</span>
      <p className="min-w-0 flex-1">
        Доступне оновлення <b>{update.version}</b>
        {error && <span className="ml-2 text-rose-300">— {error}</span>}
      </p>
      <button className="add-button h-9 px-4 text-xs" disabled={installing} onClick={() => void install()}>
        {installing ? <LoaderCircle className="animate-spin" size={14} /> : <Download size={14} />}
        {installing ? "Встановлюємо…" : "Оновити зараз"}
      </button>
      <button className="text-cyan-200/60 hover:text-cyan-100" onClick={() => setDismissed(true)}>
        <X size={16} />
      </button>
    </div>
  );
}
