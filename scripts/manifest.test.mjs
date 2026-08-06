import { describe, expect, it } from "vitest";
import { classify, isDownloadable } from "./generate-download-manifest.mjs";

describe("download manifest classification", () => {
  it("detects the platform by extension, not by a word in the name", () => {
    // Neither of these contains a platform word anywhere.
    expect(classify("web-radar-0.3.0-1.x86_64.rpm").platform).toBe("linux");
    expect(classify("Web.Radar.app.tar.gz").platform).toBe("macos");
    expect(classify("Web.Radar_0.3.0_amd64.AppImage").platform).toBe("linux");
    expect(classify("Web.Radar_0.3.0_x64-setup.exe").platform).toBe("windows");
    expect(classify("Web.Radar_0.3.0_x64_en-US.msi").platform).toBe("windows");
    expect(classify("Web.Radar_0.3.0_aarch64.dmg")).toMatchObject({
      platform: "macos",
      architecture: "arm64",
    });
  });

  it("keeps installer kinds apart so an MSI card cannot link to the exe", () => {
    expect(classify("Web.Radar_0.3.0_x64-setup.exe").kind).toBe("exe");
    expect(classify("Web.Radar_0.3.0_x64_en-US.msi").kind).toBe("msi");
    expect(classify("web-radar_0.3.0_amd64.deb").kind).toBe("deb");
  });

  it("drops signatures and the updater manifest", () => {
    expect(isDownloadable("Web.Radar_0.3.0_x64-setup.exe")).toBe(true);
    expect(isDownloadable("Web.Radar_0.3.0_x64-setup.exe.sig")).toBe(false);
    expect(isDownloadable("latest.json")).toBe(false);
  });
});
