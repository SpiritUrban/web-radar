import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./store";

describe("app store", () => {
  beforeEach(() => useAppStore.setState({ config: { vertices:"", edges:"", ranks:"", resultsDir:"results", rankMetric:"pagerank", targets:[] } }));
  it("updates editable config without losing other defaults", () => {
    useAppStore.getState().updateConfig({ rankMetric: "harmonic", targets: ["example.com"] });
    expect(useAppStore.getState().config).toMatchObject({ resultsDir:"results", rankMetric:"harmonic", targets:["example.com"] });
  });
});
