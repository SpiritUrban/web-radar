export type RankMetric = "pagerank" | "harmonic";

export interface AppConfig {
  vertices: string;
  edges: string;
  ranks: string;
  resultsDir: string;
  indexDir: string;
  rankMetric: RankMetric;
  targets: string[];
}

export interface ProductMetadata {
  productName: string;
  version: string;
  author: string;
  authorUrl: string;
  authorGithubUrl: string;
  repositoryUrl: string;
  siteUrl: string;
  copyright: string;
  dataSourceUrl: string;
}

// --- index ---------------------------------------------------------------

export type TierKey = "lookup" | "ranks" | "inbound";
export type TierState = "missing" | "ready" | "stale";

export interface TierStatus {
  key: TierKey;
  label: string;
  description: string;
  state: TierState;
  bytes: number;
  estimatedBytes: number;
  estimatedTempBytes: number;
  builtAt: string | null;
}

export interface SourceStatus {
  kind: "vertices" | "edges" | "ranks";
  path: string;
  sizeBytes: number;
  exists: boolean;
  /** Still gzipped on disk. */
  compressed: boolean;
  /** Whether gzip is a problem here — only files queries seek into. */
  mustBeUnpacked: boolean;
}

export interface DownloadHint {
  kind: "vertices" | "edges" | "ranks";
  purpose: string;
  fileName: string;
  archiveName: string;
  url: string;
  downloadBytes: number;
  unpackedBytes: number;
  mustBeUnpacked: boolean;
}

export interface SetupGuide {
  crawl: string;
  crawlListUrl: string;
  files: DownloadHint[];
  totalDownloadBytes: number;
  totalResidentBytes: number;
}

export interface IndexStatus {
  root: string;
  nodeCount: number;
  edgeCount: number;
  totalBytes: number;
  freeBytes: number;
  tiers: TierStatus[];
  sources: SourceStatus[];
  blockers: string[];
  setup: SetupGuide;
}

// --- jobs ----------------------------------------------------------------

export interface JobProgress {
  runId: number;
  kind: "index" | "scan";
  stage: string;
  detail: string;
  stageDone: number;
  stageTotal: number;
  overall: number;
  bytesPerSec: number;
  etaSecs: number;
  elapsedSecs: number;
}

// --- queries -------------------------------------------------------------

export interface LinkEntry {
  domain: string;
  rank: number;
  position?: number | null;
}

export interface DomainReport {
  domain: string;
  reverseDomain: string;
  found: boolean;
  nodeId: number | null;
  metric: string;
  rank: number | null;
  position: number | null;
  inbound: LinkEntry[];
  outbound: LinkEntry[];
  inboundTotal: number;
  outboundTotal: number;
  inboundTruncated: boolean;
  outboundTruncated: boolean;
  elapsedMs: number;
  warnings: string[];
}

export interface Capabilities {
  lookup: boolean;
  outbound: boolean;
  inbound: boolean;
  ranks: boolean;
}

export interface QueryOutcome {
  report: DomainReport;
  capabilities: Capabilities;
  nodeCount: number;
}

// --- history -------------------------------------------------------------

export interface RunRecord {
  id: number;
  kind: "index" | "scan";
  startedAt: string;
  finishedAt: string | null;
  status: "running" | "completed" | "failed" | "cancelled";
  rankMetric: string;
  targets: string[];
  outputDir: string;
  error: string | null;
}

export interface SeoReportRecord {
  id: number;
  generatedAt: string;
  provider: string;
  evidenceCount: number;
}

// --- research tools ------------------------------------------------------

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
export interface ManualPageAudit {
  url: string; sourceDomain: string; status: string; httpStatus: number | null; title: string;
  hasBacklink: boolean; backlinkCount: number; anchors: string[]; relValues: string[];
  hasBrandMention: boolean; hasDomainMention: boolean; note: string;
}
export interface ManualAuditReport {
  targetDomain: string; audited: number; confirmedBacklinks: number;
  mentionsWithoutLink: number; blockedOrSkipped: number; pages: ManualPageAudit[];
}
