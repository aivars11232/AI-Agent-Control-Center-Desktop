import { describe, expect, it } from "vitest";
import type { ModelDefinition } from "./applicationState";
import {
  executableModels,
  resolveModelAvailability,
  unknownProviderRegistrySnapshot,
  type ProviderRegistrySnapshot,
} from "./providerRegistry";

function readyRegistry(
  ollamaModels: ProviderRegistrySnapshot["providers"][number]["models"] = [],
) {
  const snapshot = unknownProviderRegistrySnapshot();
  snapshot.providers = snapshot.providers.map((status) => ({
    ...status,
    availability: "ready",
    models: status.provider.id === "ollama" ? ollamaModels : [],
    message: `${status.provider.displayName} is ready.`,
  }));
  return snapshot;
}

describe("provider registry model availability", () => {
  it("accepts an OpenAI catalog model only through a ready active Codex adapter", () => {
    const models: ModelDefinition[] = [
      { id: 1, name: "gpt-fixture", provider: "OpenAI" },
    ];

    expect(
      resolveModelAvailability(models, "gpt-fixture", readyRegistry(), "codex"),
    ).toMatchObject({
      eligible: true,
      providerId: "codex",
      reason: "Configured for Codex; the CLI validates the model at run start.",
    });
  });

  it("fails closed when the catalog provider has no runtime adapter", () => {
    const models: ModelDefinition[] = [
      { id: 2, name: "custom-fixture", provider: "Custom" },
    ];

    expect(
      resolveModelAvailability(
        models,
        "custom-fixture",
        readyRegistry(),
        "codex",
      ),
    ).toMatchObject({
      eligible: false,
      providerId: null,
      reason:
        "No executable runtime adapter is registered for this catalog provider.",
    });
  });

  it("does not silently route a model through a non-active provider", () => {
    const models: ModelDefinition[] = [
      { id: 3, name: "gpt-fixture", provider: "OpenAI" },
    ];

    expect(
      resolveModelAvailability(models, "gpt-fixture", readyRegistry(), "ollama"),
    ).toMatchObject({
      eligible: false,
      providerId: "codex",
      reason:
        "gpt-fixture runs through codex, but ollama is the active AI provider.",
    });
  });

  it("requires the active provider to report ready", () => {
    const models: ModelDefinition[] = [
      { id: 4, name: "gpt-fixture", provider: "OpenAI" },
    ];
    const snapshot = unknownProviderRegistrySnapshot("Readiness is unknown.");

    expect(
      resolveModelAvailability(models, "gpt-fixture", snapshot, "codex"),
    ).toMatchObject({
      eligible: false,
      providerId: "codex",
      reason: "Readiness is unknown.",
    });
  });

  it("requires an installed tool-capable Ollama model", () => {
    const models: ModelDefinition[] = [
      { id: 5, name: "qwen-fixture", provider: "Ollama" },
    ];
    const toolRegistry = readyRegistry([
      {
        name: "qwen-fixture",
        capabilities: ["completion", "tools"],
        contextLength: 65_536,
        availability: "ready",
        message: "Model metadata ready.",
      },
    ]);
    const llmOnlyRegistry = readyRegistry([
      {
        name: "qwen-fixture",
        capabilities: ["completion"],
        contextLength: 65_536,
        availability: "ready",
        message: "Model metadata ready.",
      },
    ]);

    expect(
      resolveModelAvailability(models, "qwen-fixture", toolRegistry, "ollama"),
    ).toMatchObject({ eligible: true, providerId: "ollama" });
    expect(
      resolveModelAvailability(
        models,
        "qwen-fixture",
        llmOnlyRegistry,
        "ollama",
      ),
    ).toMatchObject({
      eligible: false,
      reason:
        "The Ollama model qwen-fixture does not report required tool support.",
    });
    expect(
      resolveModelAvailability(
        models,
        "qwen-fixture",
        readyRegistry(),
        "ollama",
      ),
    ).toMatchObject({
      eligible: false,
      reason: "The Ollama model qwen-fixture is not installed.",
    });

    const unavailableRegistry = readyRegistry([
      {
        name: "qwen-fixture",
        capabilities: [],
        contextLength: null,
        availability: "unavailable",
        message: "Ollama returned no model metadata.",
      },
    ]);
    expect(
      resolveModelAvailability(
        models,
        "qwen-fixture",
        unavailableRegistry,
        "ollama",
      ),
    ).toMatchObject({
      eligible: false,
      reason: "Ollama returned no model metadata.",
    });
  });

  it("excludes ambiguous catalog and registry identities", () => {
    const models: ModelDefinition[] = [
      { id: 6, name: "duplicate", provider: "OpenAI" },
      { id: 7, name: "duplicate", provider: "OpenAI" },
    ];

    expect(executableModels(models, readyRegistry(), "codex")).toEqual([]);
    expect(
      resolveModelAvailability(models, "duplicate", readyRegistry(), "codex"),
    ).toMatchObject({
      eligible: false,
      reason: "The selected model name is ambiguous in the model catalog.",
    });

    const singleModel = [models[0]];
    const ambiguousRegistry = readyRegistry();
    const openAiBinding = ambiguousRegistry.catalogBindings.find(
      (binding) => binding.catalogProvider === "OpenAI",
    );
    expect(openAiBinding).toBeDefined();
    ambiguousRegistry.catalogBindings.push({ ...openAiBinding! });
    expect(
      resolveModelAvailability(
        singleModel,
        "duplicate",
        ambiguousRegistry,
        "codex",
      ),
    ).toMatchObject({
      eligible: false,
      reason:
        "The catalog provider binding is ambiguous in the runtime registry.",
    });
  });
});
