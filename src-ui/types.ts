export type RankMetric = "pagerank" | "harmonic";

export interface AppConfig {
  vertices: string;
  edges: string;
  ranks: string;
  resultsDir: string;
  rankMetric: RankMetric;
  targets: string[];
}

export interface FileIndex {
  kind: string;
  path: string;
  sizeBytes: number;
  modifiedAt: number | null;
  exists: boolean;
}

export interface RunRecord {
  id: number;
  startedAt: string;
  finishedAt: string | null;
  status: "running" | "completed" | "failed";
  rankMetric: string;
  targets: string[];
  resultsDir: string;
  error: string | null;
}

export interface ProgressEvent {
  runId: number;
  phase: string;
  message: string;
  progress: number;
}
