import { describe, expect, it } from "vitest";
import appStyles from "../App.css?inline";
import responsiveStyles from "./responsive.css?inline";

describe("responsive stylesheet contract", () => {
  it("resolves the owned stylesheet modules into the application bundle", () => {
    expect(appStyles).toContain(":root {");
    expect(appStyles).toContain(".app-shell {");
    expect(appStyles).toContain(".dashboard-group-tabs {");
    expect(appStyles).toContain(".settings-block {");
    expect(appStyles).toContain(".workspace-evidence {");
    expect(appStyles).toContain("@media (max-width: 520px)");
  });

  it("keeps provider status and selection visible at narrow widths", () => {
    expect(responsiveStyles).toMatch(
      /@media \(max-width: 520px\)[\s\S]*?\.system-status\s*\{[^}]*display:\s*flex;/,
    );
    expect(responsiveStyles).not.toMatch(
      /\.system-status\s*\{[^}]*display:\s*none;/,
    );
  });

  it("honors both the application and operating-system reduced-motion preferences", () => {
    expect(responsiveStyles).toContain(':root[data-motion="reduced"] *');
    expect(responsiveStyles).toContain(
      "@media (prefers-reduced-motion: reduce)",
    );
    expect(responsiveStyles).toContain("transition-duration: 0.001ms !important");
    expect(responsiveStyles).toContain("animation-duration: 0.001ms !important");
  });
});
