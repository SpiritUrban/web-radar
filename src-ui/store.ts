import { create } from "zustand";
import type {
  AppConfig, IndexStatus, JobProgress, ProductMetadata, QueryOutcome, RunRecord,
} from "./types";

const emptyConfig: AppConfig = {
  vertices: "",
  edges: "",
  ranks: "",
  resultsDir: "results",
  indexDir: "index",
  rankMetric: "pagerank",
  targets: [],
};

interface AppState {
  config: AppConfig;
  /** Config as last read from disk — used to show "unsaved changes". */
  savedConfig: AppConfig;
  metadata: ProductMetadata | null;
  index: IndexStatus | null;
  history: RunRecord[];
  job: JobProgress | null;
  /** Which long job is running, if any. */
  busy: null | "index" | "scan" | "query";
  result: QueryOutcome | null;
  error: string | null;
  notice: string | null;

  setConfig: (config: AppConfig) => void;
  updateConfig: (patch: Partial<AppConfig>) => void;
  markConfigSaved: () => void;
  setMetadata: (metadata: ProductMetadata) => void;
  setIndex: (index: IndexStatus | null) => void;
  setHistory: (history: RunRecord[]) => void;
  setJob: (job: JobProgress | null) => void;
  setBusy: (busy: AppState["busy"]) => void;
  setResult: (result: QueryOutcome | null) => void;
  setError: (error: string | null) => void;
  setNotice: (notice: string | null) => void;
}

export const useAppStore = create<AppState>((set) => ({
  config: emptyConfig,
  savedConfig: emptyConfig,
  metadata: null,
  index: null,
  history: [],
  job: null,
  busy: null,
  result: null,
  error: null,
  notice: null,

  setConfig: (config) => set({ config, savedConfig: config }),
  updateConfig: (patch) => set((state) => ({ config: { ...state.config, ...patch } })),
  markConfigSaved: () => set((state) => ({ savedConfig: state.config })),
  setMetadata: (metadata) => set({ metadata }),
  setIndex: (index) => set({ index }),
  setHistory: (history) => set({ history }),
  setJob: (job) => set({ job }),
  setBusy: (busy) => set({ busy }),
  setResult: (result) => set({ result }),
  setError: (error) => set({ error }),
  setNotice: (notice) => set({ notice }),
}));

/** True when the edited config differs from what is on disk. */
export function hasUnsavedChanges(state: { config: AppConfig; savedConfig: AppConfig }): boolean {
  return JSON.stringify(state.config) !== JSON.stringify(state.savedConfig);
}

/** Tier lookup that tolerates a missing index status. */
export function tierState(index: IndexStatus | null, key: string) {
  return index?.tiers.find((tier) => tier.key === key) ?? null;
}
