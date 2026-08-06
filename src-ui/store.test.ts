import { beforeEach, describe, expect, it } from "vitest";
import { hasUnsavedChanges, tierState, useAppStore } from "./store";
import { cleanDomain, deriveBrand, formatBytes, formatCount, formatRank } from "./format";
import type { AppConfig, IndexStatus } from "./types";

const baseConfig: AppConfig = {
  vertices: "v.txt", edges: "e.txt", ranks: "r.txt",
  resultsDir: "results", indexDir: "index", rankMetric: "pagerank", targets: [],
};

describe("app store", () => {
  beforeEach(() => useAppStore.setState({ config: baseConfig, savedConfig: baseConfig }));

  it("updates editable config without losing other defaults", () => {
    useAppStore.getState().updateConfig({ rankMetric: "harmonic", targets: ["example.com"] });
    expect(useAppStore.getState().config).toMatchObject({
      resultsDir: "results", indexDir: "index", rankMetric: "harmonic", targets: ["example.com"],
    });
  });

  it("tracks unsaved changes so the Save button can mean something", () => {
    expect(hasUnsavedChanges(useAppStore.getState())).toBe(false);
    useAppStore.getState().updateConfig({ indexDir: "D:/web-radar-index" });
    expect(hasUnsavedChanges(useAppStore.getState())).toBe(true);
    useAppStore.getState().markConfigSaved();
    expect(hasUnsavedChanges(useAppStore.getState())).toBe(false);
  });

  it("survives a missing index status instead of throwing in render", () => {
    expect(tierState(null, "inbound")).toBeNull();
    const status = { tiers: [{ key: "inbound", state: "ready" }] } as unknown as IndexStatus;
    expect(tierState(status, "inbound")?.state).toBe("ready");
    expect(tierState(status, "ranks")).toBeNull();
  });
});

describe("formatting", () => {
  it("formats sizes the way the status bar shows them", () => {
    expect(formatBytes(0)).toBe("—");
    expect(formatBytes(2048)).toBe("2 КБ");
    expect(formatBytes(16 * 1024 ** 3)).toBe("16.0 ГБ");
  });

  it("keeps huge counts readable", () => {
    expect(formatCount(121091933)).toMatch(/121\D091\D933/);
    expect(formatCount(null)).toBe("—");
  });

  it("shows tiny ranks in scientific notation rather than as zero", () => {
    expect(formatRank(0)).toBe("—");
    expect(formatRank(4.115e-9)).toContain("e-9");
    expect(formatRank(0.25)).toBe("0.2500");
  });

  it("normalises whatever the user pastes into a hostname", () => {
    expect(cleanDomain("https://WWW.Example.com/page?x=1")).toBe("example.com");
    expect(cleanDomain("  my-transfer.com.ua.  ")).toBe("my-transfer.com.ua");
    expect(cleanDomain("example.com:8443")).toBe("example.com");
    expect(deriveBrand("https://my-transfer.com.ua/")).toBe("My Transfer");
  });
});
