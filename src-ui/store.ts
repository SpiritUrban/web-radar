import { create } from "zustand";
import type { AppConfig, FileIndex, ProgressEvent, RunRecord } from "./types";

const emptyConfig: AppConfig = {
  vertices: "",
  edges: "",
  ranks: "",
  resultsDir: "results",
  rankMetric: "pagerank",
  targets: [],
};

interface AppState {
  config: AppConfig;
  history: RunRecord[];
  files: FileIndex[];
  running: boolean;
  progress: ProgressEvent | null;
  error: string | null;
  setConfig: (config: AppConfig) => void;
  updateConfig: (patch: Partial<AppConfig>) => void;
  setHistory: (history: RunRecord[]) => void;
  setFiles: (files: FileIndex[]) => void;
  setRunning: (running: boolean) => void;
  setProgress: (progress: ProgressEvent | null) => void;
  setError: (error: string | null) => void;
}

export const useAppStore = create<AppState>((set) => ({
  config: emptyConfig,
  history: [],
  files: [],
  running: false,
  progress: null,
  error: null,
  setConfig: (config) => set({ config }),
  updateConfig: (patch) => set((state) => ({ config: { ...state.config, ...patch } })),
  setHistory: (history) => set({ history }),
  setFiles: (files) => set({ files }),
  setRunning: (running) => set({ running }),
  setProgress: (progress) => set({ progress }),
  setError: (error) => set({ error }),
}));
