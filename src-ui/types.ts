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
  processedBytes?: number;
  totalBytes?: number;
  elapsedSecs?: number;
}
export interface SeoTarget { domain: string; brand: string; }
export interface SeoEvidence {
  targetDomain: string; queryKind: string; query: string; title: string; url: string;
  sourceDomain: string; snippet: string; evidenceType: string; confidence: string;
}
export interface SeoInsight { level: string; title: string; detail: string; }
export interface SeoQuery { kind: string; query: string; purpose: string; }
export interface SeoDiscoveryReport {
  provider: string; generatedAt: string; queries: SeoQuery[]; evidence: SeoEvidence[];
  insights: SeoInsight[]; uniqueSourceDomains: number; potentialBacklinks: number;
  brandMentions: number; reputationMentions: number;
}
