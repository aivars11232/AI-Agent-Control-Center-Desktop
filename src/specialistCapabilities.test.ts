import { describe, expect, it } from "vitest";
import {
  buildSpecialistTaskRequest,
  createSpecialistTaskDraft,
  specialistProfileForTemplate,
} from "./specialistCapabilities";

describe("TASK-0017 typed specialist task composition", () => {
  it("builds distinct deterministic requests for all four stable templates", () => {
    const coding = createSpecialistTaskDraft();
    coding.acceptanceCriteria = "Focused check passes";
    coding.mutationClasses = ["modify"];
    expect(buildSpecialistTaskRequest("coding", "Implement fix", coding).request).toMatchObject({
      kind: "coding",
      objective: "Implement fix",
      mutationClasses: ["modify"],
    });

    const debugging = createSpecialistTaskDraft();
    debugging.symptoms = "Test exits with code 1";
    debugging.expectedBehavior = "Test passes";
    expect(buildSpecialistTaskRequest("debugging", "Find cause", debugging).request).toMatchObject({
      kind: "debugging",
      objective: "Find cause",
      symptoms: ["Test exits with code 1"],
    });

    const browser = createSpecialistTaskDraft();
    browser.allowedDomains = "KDE.org, freedesktop.org";
    expect(buildSpecialistTaskRequest("browser", "Find native APIs", browser).request).toMatchObject({
      kind: "browserResearch",
      question: "Find native APIs",
      allowedDomains: ["kde.org", "freedesktop.org"],
      maxSources: 5,
    });

    const financial = createSpecialistTaskDraft();
    financial.currency = "eur";
    financial.calculations = "margin | percentOf | 2500.00, 12.5 | 2";
    expect(buildSpecialistTaskRequest("financial", "Calculate margin", financial).request).toMatchObject({
      kind: "financialAnalysis",
      currency: "EUR",
      calculations: [
        {
          id: "margin",
          operation: "percentOf",
          operands: ["2500.00", "12.5"],
          outputScale: 2,
        },
      ],
    });
  });

  it("keeps generic agents untyped and rejects incomplete or malformed specialist drafts", () => {
    const draft = createSpecialistTaskDraft();
    expect(buildSpecialistTaskRequest(null, "Generic task", draft)).toEqual({
      request: null,
      error: null,
    });
    expect(buildSpecialistTaskRequest("coding", "Change code", draft).error).toContain(
      "acceptance criterion",
    );

    draft.calculations = "bad | trade | 1, 2 | 2";
    expect(buildSpecialistTaskRequest("financial", "Analyze", draft).error).toContain(
      "unsupported operation",
    );
  });

  it("states each effective role ceiling in the renderer profile", () => {
    expect(specialistProfileForTemplate("coding")?.summary).toContain("one-use-approved");
    expect(specialistProfileForTemplate("debugging")?.summary).toContain("Read-only");
    expect(specialistProfileForTemplate("browser")?.ceilings.join(" ")).toContain("No interactive browser");
    expect(specialistProfileForTemplate("financial")?.ceilings.join(" ")).toContain("No trading");
  });
});

