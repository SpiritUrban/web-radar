import { describe, expect, it } from "vitest";
import { formatDuration, metricLabel, plural } from "./format";

describe("ukrainian plurals", () => {
  const domains = (n: number) => `${n} ${plural(n, "домен", "домени", "доменів")}`;

  it("uses the right form for each group", () => {
    expect(domains(1)).toBe("1 домен");
    expect(domains(2)).toBe("2 домени");
    expect(domains(4)).toBe("4 домени");
    expect(domains(5)).toBe("5 доменів");
    expect(domains(0)).toBe("0 доменів");
  });

  it("handles the teens, which do not follow the last digit", () => {
    expect(domains(11)).toBe("11 доменів");
    expect(domains(12)).toBe("12 доменів");
    expect(domains(14)).toBe("14 доменів");
    expect(domains(21)).toBe("21 домен");
    expect(domains(22)).toBe("22 домени");
    expect(domains(112)).toBe("112 доменів");
  });
});

describe("labels", () => {
  it("spells metric names the way their owners do", () => {
    expect(metricLabel("pagerank")).toBe("PageRank");
    expect(metricLabel("Pagerank")).toBe("PageRank");
    expect(metricLabel("harmonic")).toBe("Harmonic");
  });

  it("formats durations without turning minutes into decimals", () => {
    expect(formatDuration(45)).toBe("45 с");
    expect(formatDuration(90)).toBe("1 хв 30 с");
    expect(formatDuration(7_800)).toBe("2 год 10 хв");
  });
});
