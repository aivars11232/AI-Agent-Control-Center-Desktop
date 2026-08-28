export const SPECIALIST_SCHEMA_VERSION = 1 as const;
export const SPECIALIST_PROFILE_VERSION = "specialist-profile-v1" as const;

export type SpecialistTemplateKey =
  | "coding"
  | "debugging"
  | "browser"
  | "financial";

export type WorkspaceMutationClass =
  | "create"
  | "modify"
  | "delete"
  | "rename";

export type FinancialOperation =
  | "sum"
  | "difference"
  | "product"
  | "quotient"
  | "percentOf"
  | "percentChange";

export type FinancialCalculation = {
  id: string;
  operation: FinancialOperation;
  operands: string[];
  outputScale: number;
};

export type SpecialistTaskRequest =
  | {
      kind: "coding";
      schemaVersion: 1;
      profileVersion: typeof SPECIALIST_PROFILE_VERSION;
      objective: string;
      acceptanceCriteria: string[];
      constraints: string[];
      mutationClasses: WorkspaceMutationClass[];
      requestedChecks: string[];
      allowWebResearch: boolean;
    }
  | {
      kind: "debugging";
      schemaVersion: 1;
      profileVersion: typeof SPECIALIST_PROFILE_VERSION;
      objective: string;
      symptoms: string[];
      expectedBehavior: string;
      reproductionSteps: string[];
      requestedChecks: string[];
    }
  | {
      kind: "browserResearch";
      schemaVersion: 1;
      profileVersion: typeof SPECIALIST_PROFILE_VERSION;
      question: string;
      allowedDomains: string[];
      maxSources: number;
      freshnessContext: string | null;
    }
  | {
      kind: "financialAnalysis";
      schemaVersion: 1;
      profileVersion: typeof SPECIALIST_PROFILE_VERSION;
      question: string;
      currency: string | null;
      assumptions: string[];
      calculations: FinancialCalculation[];
    };

export type SpecialistToolContract = {
  workspace: string;
  terminal: string;
  internet: string;
  calculator: string;
  externalEffects: string;
};

export type SpecialistRunContract = {
  schemaVersion: number;
  contractVersion: string;
  profileVersion: string;
  kind: "coding" | "debugging" | "browserResearch" | "financialAnalysis";
  templateKey: SpecialistTemplateKey;
  requestSha256: string;
  workspaceBinding: string;
  tools: SpecialistToolContract;
  approvalClass: string;
  approvalId: number | null;
  provider: string;
  model: string;
};

export type SpecialistCheckResult = {
  command: string;
  status: string;
  summary: string;
};

export type SpecialistResult =
  | {
      kind: "coding";
      summary: string;
      changes: string[];
      verification: SpecialistCheckResult[];
      evidenceRefs: Array<{ kind: string; reference: string }>;
      limitations: string[];
    }
  | {
      kind: "debugging";
      summary: string;
      findings: string[];
      rootCauses: string[];
      reproduction: string[];
      recommendedFixes: string[];
      checks: SpecialistCheckResult[];
      workspaceChanged: boolean;
    }
  | {
      kind: "browserResearch";
      answer: string;
      sources: Array<{
        title: string;
        url: string;
        retrievedAt: string;
        supports: string;
      }>;
      limitations: string[];
      externalEffects: string[];
    }
  | {
      kind: "financialAnalysis";
      report: string;
      calculationResults: Array<{ id: string; value: string }>;
      assumptions: string[];
      caveats: string[];
      decisionAuthority: string;
      externalEffects: string[];
    };

export type SpecialistTaskDraft = {
  acceptanceCriteria: string;
  constraints: string;
  mutationClasses: WorkspaceMutationClass[];
  requestedChecks: string;
  allowWebResearch: boolean;
  symptoms: string;
  expectedBehavior: string;
  reproductionSteps: string;
  allowedDomains: string;
  maxSources: number;
  freshnessContext: string;
  currency: string;
  assumptions: string;
  calculations: string;
};

export type SpecialistProfile = {
  templateKey: SpecialistTemplateKey;
  label: string;
  category: "Development" | "Browsing" | "Finance";
  primaryLabel: string;
  summary: string;
  ceilings: string[];
};

const specialistProfiles: Record<SpecialistTemplateKey, SpecialistProfile> = {
  coding: {
    templateKey: "coding",
    label: "Coding",
    category: "Development",
    primaryLabel: "Implementation objective",
    summary: "A bounded, one-use-approved implementation inside the selected workspace.",
    ceilings: [
      "Observed workspace mutations must stay within the declared create, modify, delete, or rename classes",
      "The result must report the exact requested checks; Codex safe terminal remains a sandbox ceiling",
      "Ollama supports declared create/modify only and rejects checks, delete, or rename contracts",
      "Hosted web research only when explicitly enabled",
      "No system control, credentials, accounts, purchases, or external submissions",
    ],
  },
  debugging: {
    templateKey: "debugging",
    label: "Debugging",
    category: "Development",
    primaryLabel: "Diagnosis objective",
    summary: "Read-only diagnosis and requested checks; fixes require a separate Coding task.",
    ceilings: [
      "Selected workspace is read-only",
      "Terminal is disabled when no checks are requested; the result must report the exact requested list",
      "Codex safe terminal is sandboxed rather than command-intercepted; Ollama cannot run requested checks",
      "No web, file edits, formatting, fixes, or external effects",
      "Senior Coding review binds to the stable Debugging template",
    ],
  },
  browser: {
    templateKey: "browser",
    label: "Browser Research",
    category: "Browsing",
    primaryLabel: "Research question",
    summary: "Hosted read-only research with bounded sources and a disposable private scratch area.",
    ceilings: [
      "HTTPS sources only, optionally restricted to listed domains",
      "No interactive browser, authentication, forms, uploads, or downloads",
      "The selected user workspace is not the run workspace; Codex uses private read-only scratch",
      "The current Codex CLI cannot enforce literal no-file access; purchases, account changes, and other external effects remain prohibited",
    ],
  },
  financial: {
    templateKey: "financial",
    label: "Financial Analysis",
    category: "Finance",
    primaryLabel: "Analysis question",
    summary: "Local read-only reporting with backend fixed-point calculations and user decision authority.",
    ceilings: [
      "No selected-user-workspace binding, web, shell, account, or credential authority",
      "Ollama receives zero tools; Codex uses private read-only scratch because literal no-file access is unsupported",
      "No trading, transfers, purchases, submissions, or autonomous decisions",
      "Declared calculations use bounded decimal inputs and half-even rounding",
      "The structured result must leave decision authority with the user",
    ],
  },
};

const financialOperations = new Set<FinancialOperation>([
  "sum",
  "difference",
  "product",
  "quotient",
  "percentOf",
  "percentChange",
]);

export function createSpecialistTaskDraft(): SpecialistTaskDraft {
  return {
    acceptanceCriteria: "",
    constraints: "",
    mutationClasses: ["modify"],
    requestedChecks: "",
    allowWebResearch: false,
    symptoms: "",
    expectedBehavior: "",
    reproductionSteps: "",
    allowedDomains: "",
    maxSources: 5,
    freshnessContext: "",
    currency: "",
    assumptions: "",
    calculations: "",
  };
}

export function specialistTemplateKey(
  value: string | null | undefined,
): SpecialistTemplateKey | null {
  return value === "coding" ||
    value === "debugging" ||
    value === "browser" ||
    value === "financial"
    ? value
    : null;
}

export function specialistProfileForTemplate(
  value: string | null | undefined,
): SpecialistProfile | null {
  const key = specialistTemplateKey(value);
  return key ? specialistProfiles[key] : null;
}

function textLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function domains(value: string): string[] {
  return value
    .split(/[\r\n,]+/)
    .map((item) => item.trim().toLowerCase())
    .filter(Boolean);
}

function parseFinancialCalculations(
  value: string,
): { calculations: FinancialCalculation[]; error: string | null } {
  const calculations: FinancialCalculation[] = [];
  const ids = new Set<string>();
  for (const [index, line] of textLines(value).entries()) {
    const parts = line.split("|").map((item) => item.trim());
    if (parts.length !== 4) {
      return {
        calculations: [],
        error: `Calculation line ${index + 1} must use: id | operation | operand, operand | scale.`,
      };
    }
    const [id, operationText, operandsText, scaleText] = parts;
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id) || ids.has(id)) {
      return {
        calculations: [],
        error: `Calculation line ${index + 1} needs a unique 1–64 character ASCII id.`,
      };
    }
    if (!financialOperations.has(operationText as FinancialOperation)) {
      return {
        calculations: [],
        error: `Calculation line ${index + 1} has an unsupported operation.`,
      };
    }
    const operation = operationText as FinancialOperation;
    const operands = operandsText
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
    if ((operation === "sum" && operands.length < 1) ||
        (operation !== "sum" && operands.length !== 2)) {
      return {
        calculations: [],
        error: `Calculation line ${index + 1} has the wrong operand count.`,
      };
    }
    const outputScale = Number(scaleText);
    if (!Number.isInteger(outputScale) || outputScale < 0 || outputScale > 12) {
      return {
        calculations: [],
        error: `Calculation line ${index + 1} needs an integer scale from 0 to 12.`,
      };
    }
    ids.add(id);
    calculations.push({ id, operation, operands, outputScale });
  }
  return { calculations, error: null };
}

export function buildSpecialistTaskRequest(
  templateKey: string | null | undefined,
  primaryText: string,
  draft: SpecialistTaskDraft,
): { request: SpecialistTaskRequest | null; error: string | null } {
  const key = specialistTemplateKey(templateKey);
  if (!key) return { request: null, error: null };
  const objective = primaryText.trim();
  if (!objective) {
    return { request: null, error: "Enter the role-specific objective or question." };
  }

  if (key === "coding") {
    const acceptanceCriteria = textLines(draft.acceptanceCriteria);
    if (acceptanceCriteria.length === 0) {
      return { request: null, error: "Coding requires at least one acceptance criterion." };
    }
    if (draft.mutationClasses.length === 0) {
      return { request: null, error: "Coding requires at least one mutation class." };
    }
    return {
      request: {
        kind: "coding",
        schemaVersion: SPECIALIST_SCHEMA_VERSION,
        profileVersion: SPECIALIST_PROFILE_VERSION,
        objective,
        acceptanceCriteria,
        constraints: textLines(draft.constraints),
        mutationClasses: [...draft.mutationClasses],
        requestedChecks: textLines(draft.requestedChecks),
        allowWebResearch: draft.allowWebResearch,
      },
      error: null,
    };
  }

  if (key === "debugging") {
    const symptoms = textLines(draft.symptoms);
    if (symptoms.length === 0 || !draft.expectedBehavior.trim()) {
      return {
        request: null,
        error: "Debugging requires at least one symptom and the expected behavior.",
      };
    }
    return {
      request: {
        kind: "debugging",
        schemaVersion: SPECIALIST_SCHEMA_VERSION,
        profileVersion: SPECIALIST_PROFILE_VERSION,
        objective,
        symptoms,
        expectedBehavior: draft.expectedBehavior.trim(),
        reproductionSteps: textLines(draft.reproductionSteps),
        requestedChecks: textLines(draft.requestedChecks),
      },
      error: null,
    };
  }

  if (key === "browser") {
    if (!Number.isInteger(draft.maxSources) || draft.maxSources < 1 || draft.maxSources > 20) {
      return { request: null, error: "Browser Research max sources must be from 1 to 20." };
    }
    return {
      request: {
        kind: "browserResearch",
        schemaVersion: SPECIALIST_SCHEMA_VERSION,
        profileVersion: SPECIALIST_PROFILE_VERSION,
        question: objective,
        allowedDomains: domains(draft.allowedDomains),
        maxSources: draft.maxSources,
        freshnessContext: draft.freshnessContext.trim() || null,
      },
      error: null,
    };
  }

  const currency = draft.currency.trim().toUpperCase();
  if (currency && !/^[A-Z]{3}$/.test(currency)) {
    return { request: null, error: "Currency must be a three-letter code such as EUR." };
  }
  const parsed = parseFinancialCalculations(draft.calculations);
  if (parsed.error) return { request: null, error: parsed.error };
  return {
    request: {
      kind: "financialAnalysis",
      schemaVersion: SPECIALIST_SCHEMA_VERSION,
      profileVersion: SPECIALIST_PROFILE_VERSION,
      question: objective,
      currency: currency || null,
      assumptions: textLines(draft.assumptions),
      calculations: parsed.calculations,
    },
    error: null,
  };
}

export function isSpecialistTaskRequest(value: unknown): value is SpecialistTaskRequest {
  if (!value || typeof value !== "object") return false;
  const request = value as Partial<SpecialistTaskRequest>;
  return request.schemaVersion === SPECIALIST_SCHEMA_VERSION &&
    request.profileVersion === SPECIALIST_PROFILE_VERSION &&
    (request.kind === "coding" ||
      request.kind === "debugging" ||
      request.kind === "browserResearch" ||
      request.kind === "financialAnalysis");
}
