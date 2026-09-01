import { describe, expect, it } from "vitest";
import appStyles from "../App.css?inline";
import responsiveStyles from "./responsive.css?inline";
import shellStyles from "./shell.css?inline";
import sharedStyles from "./shared-components.css?inline";

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

describe("desktop shell layout contract", () => {
  it("gives the sidebar provider control the full sidebar width", () => {
    // The label and the select previously shared one flex row inside a ~230px
    // sidebar, which truncated the select's own value to "Co".
    expect(shellStyles).toMatch(
      /\.system-provider-select\s*\{[^}]*display:\s*grid;/,
    );
    expect(shellStyles).not.toMatch(
      /\.system-provider-select\s*\{[^}]*display:\s*flex;/,
    );
    expect(shellStyles).toMatch(
      /\.system-provider-select select\s*\{[^}]*width:\s*100%;/,
    );
  });

  it("keeps the provider status hint readable instead of ellipsising it", () => {
    expect(shellStyles).toMatch(
      /\.system-status small\s*\{[^}]*overflow-wrap:\s*break-word;/,
    );
    expect(shellStyles).not.toMatch(
      /\.system-status strong,\s*\n\.system-status small\s*\{[^}]*white-space:\s*nowrap;/,
    );
  });

  it("lays the persistence status screen out on its own full-width shell", () => {
    // `.app-shell` reserves its first grid column for the sidebar, so the
    // recovery screen must not reuse it with a single child.
    expect(shellStyles).toMatch(/\.status-shell\s*\{[^}]*display:\s*flex;/);
    expect(shellStyles).toMatch(
      /\.status-main\s*\{[^}]*justify-content:\s*center;/,
    );
    expect(shellStyles).toMatch(
      /\.status-detail\s*\{[^}]*overflow-wrap:\s*break-word;/,
    );
  });

  it("stacks adjacent card captions and scales long identifier values", () => {
    expect(sharedStyles).toMatch(/\.agent-card small\s*\{[^}]*display:\s*block;/);
    expect(sharedStyles).toMatch(
      /\.summary-card strong\.summary-value-text\s*\{[^}]*overflow-wrap:\s*break-word;/,
    );
  });
});
