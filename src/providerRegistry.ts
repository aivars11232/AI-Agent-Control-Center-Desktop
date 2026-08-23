import type {
  ModelDefinition,
  ModelProvider,
  RuntimeProviderId,
} from "./applicationState";

export type ProviderAvailability = "ready" | "unavailable" | "unknown";

export type ProviderCapabilities = {
  workspaceRead: boolean;
  workspaceWrite: boolean;
  webSearch: boolean;
  workspaceTools: boolean;
  usageReporting: boolean;
};

export type ProviderDescriptor = {
  id: RuntimeProviderId;
  displayName: string;
  capabilities: ProviderCapabilities;
};

export type ProviderRuntimeModel = {
  name: string;
  capabilities: string[];
  contextLength: number | null;
};

export type ProviderRuntimeStatus = {
  provider: ProviderDescriptor;
  availability: ProviderAvailability;
  version: string | null;
  models: ProviderRuntimeModel[];
  message: string;
};

export type CatalogProviderBinding = {
  catalogProvider: ModelProvider;
  providerId: RuntimeProviderId | null;
  adapterAvailable: boolean;
  message: string;
};

export type ProviderRegistrySnapshot = {
  providers: ProviderRuntimeStatus[];
  catalogBindings: CatalogProviderBinding[];
};

export type ModelAvailability = {
  eligible: boolean;
  model: ModelDefinition | null;
  providerId: RuntimeProviderId | null;
  runtimeModel: ProviderRuntimeModel | null;
  reason: string;
};

const providerCapabilities: Record<RuntimeProviderId, ProviderCapabilities> = {
  codex: {
    workspaceRead: true,
    workspaceWrite: true,
    webSearch: true,
    workspaceTools: false,
    usageReporting: false,
  },
  ollama: {
    workspaceRead: true,
    workspaceWrite: true,
    webSearch: false,
    workspaceTools: true,
    usageReporting: true,
  },
};

const catalogBindings: CatalogProviderBinding[] = [
  {
    catalogProvider: "OpenAI",
    providerId: "codex",
    adapterAvailable: true,
    message: "Runs through the installed Codex CLI.",
  },
  {
    catalogProvider: "Anthropic",
    providerId: null,
    adapterAvailable: false,
    message:
      "No executable runtime adapter is registered for this catalog provider.",
  },
  {
    catalogProvider: "Google",
    providerId: null,
    adapterAvailable: false,
    message:
      "No executable runtime adapter is registered for this catalog provider.",
  },
  {
    catalogProvider: "Ollama",
    providerId: "ollama",
    adapterAvailable: true,
    message: "Runs through the local Ollama service.",
  },
  {
    catalogProvider: "Custom",
    providerId: null,
    adapterAvailable: false,
    message:
      "No executable runtime adapter is registered for this catalog provider.",
  },
];

export function unknownProviderRegistrySnapshot(
  message = "Provider readiness has not been inspected.",
): ProviderRegistrySnapshot {
  return {
    providers: (["codex", "ollama"] as RuntimeProviderId[]).map((id) => ({
      provider: {
        id,
        displayName: id === "codex" ? "Codex" : "Ollama",
        capabilities: providerCapabilities[id],
      },
      availability: "unknown",
      version: null,
      models: [],
      message,
    })),
    catalogBindings: catalogBindings.map((binding) => ({ ...binding })),
  };
}

export function providerRuntimeStatus(
  snapshot: ProviderRegistrySnapshot,
  providerId: RuntimeProviderId,
): ProviderRuntimeStatus | null {
  const matches = snapshot.providers.filter(
    (status) => status.provider.id === providerId,
  );
  return matches.length === 1 ? matches[0] : null;
}

function unavailable(
  reason: string,
  model: ModelDefinition | null = null,
  providerId: RuntimeProviderId | null = null,
): ModelAvailability {
  return {
    eligible: false,
    model,
    providerId,
    runtimeModel: null,
    reason,
  };
}

export function resolveModelAvailability(
  models: ModelDefinition[],
  selectedModel: string,
  snapshot: ProviderRegistrySnapshot,
  activeProvider: RuntimeProviderId,
): ModelAvailability {
  const matchingModels = models.filter(
    (model) => model.name === selectedModel,
  );
  if (matchingModels.length === 0) {
    return unavailable(
      "The selected model is not registered in the model catalog.",
    );
  }
  if (matchingModels.length > 1) {
    return unavailable(
      "The selected model name is ambiguous in the model catalog.",
    );
  }

  const model = matchingModels[0];
  const matchingBindings = snapshot.catalogBindings.filter(
    (item) => item.catalogProvider === model.provider,
  );
  if (matchingBindings.length > 1) {
    return unavailable(
      "The catalog provider binding is ambiguous in the runtime registry.",
      model,
    );
  }
  const binding = matchingBindings[0];
  if (!binding || !binding.adapterAvailable || !binding.providerId) {
    return unavailable(
      binding?.message ?? "The catalog provider is not recognized.",
      model,
    );
  }

  const providerId = binding.providerId;
  if (providerId !== activeProvider) {
    return unavailable(
      `${model.name} runs through ${providerId}, but ${activeProvider} is the active AI provider.`,
      model,
      providerId,
    );
  }

  const runtimeStatus = providerRuntimeStatus(snapshot, providerId);
  if (!runtimeStatus) {
    return unavailable(
      `The ${providerId} runtime registry entry is missing or ambiguous.`,
      model,
      providerId,
    );
  }
  if (runtimeStatus.availability !== "ready") {
    return unavailable(runtimeStatus.message, model, providerId);
  }

  if (providerId === "codex") {
    return {
      eligible: true,
      model,
      providerId,
      runtimeModel: null,
      reason: "Configured for Codex; the CLI validates the model at run start.",
    };
  }

  const runtimeMatches = runtimeStatus.models.filter((runtimeModel) =>
    runtimeModel.name.toLowerCase() === model.name.toLowerCase(),
  );
  if (runtimeMatches.length === 0) {
    return unavailable(
      `The Ollama model ${model.name} is not installed.`,
      model,
      providerId,
    );
  }
  if (runtimeMatches.length > 1) {
    return unavailable(
      `The Ollama model ${model.name} is ambiguous in runtime discovery.`,
      model,
      providerId,
    );
  }

  const runtimeModel = runtimeMatches[0];
  if (
    !runtimeModel.capabilities.some(
      (capability) => capability.toLowerCase() === "tools",
    )
  ) {
    return unavailable(
      `The Ollama model ${model.name} does not report required tool support.`,
      model,
      providerId,
    );
  }

  return {
    eligible: true,
    model,
    providerId,
    runtimeModel,
    reason: "Installed Ollama model with workspace tool support.",
  };
}

export function executableModels(
  models: ModelDefinition[],
  snapshot: ProviderRegistrySnapshot,
  activeProvider: RuntimeProviderId,
): ModelDefinition[] {
  return models.filter(
    (model) =>
      resolveModelAvailability(models, model.name, snapshot, activeProvider)
        .eligible,
  );
}
