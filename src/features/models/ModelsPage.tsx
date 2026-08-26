import { useState } from "react";
import type { ModelDefinition, ModelProvider, RuntimeProviderId } from "../../applicationState";
import { providerRuntimeStatus, resolveModelAvailability } from "../../providerRegistry";
import type { ProviderRegistrySnapshot } from "../../providerRegistry";

export function ModelsPage({
  models,
  setModels,
  providerRegistry,
  activeProvider,
  registryBusy,
  registryMessage,
  onRefreshRegistry,
}: {
  models: ModelDefinition[];
  setModels: React.Dispatch<React.SetStateAction<ModelDefinition[]>>;
  providerRegistry: ProviderRegistrySnapshot;
  activeProvider: RuntimeProviderId;
  registryBusy: boolean;
  registryMessage: string;
  onRefreshRegistry: () => Promise<void>;
}) {
  const [modelName, setModelName] = useState("");
  const [provider, setProvider] = useState<ModelProvider>("OpenAI");
  const ollamaStatus = providerRuntimeStatus(providerRegistry, "ollama");
  const ollamaReady = ollamaStatus?.availability === "ready";
  const ollamaMessage = registryMessage || ollamaStatus?.message || "";
  const selectedBinding = providerRegistry.catalogBindings.find(
    (binding) => binding.catalogProvider === provider,
  );

  function addModel() {
    const trimmedName = modelName.trim();

    if (!trimmedName) {
      return;
    }

    const alreadyExists = models.some(
      (model) => model.name.toLowerCase() === trimmedName.toLowerCase(),
    );

    if (alreadyExists) {
      window.alert("A model with this name already exists.");
      return;
    }

    setModels((currentModels) => [
      ...currentModels,
      {
        id: Date.now(),
        name: trimmedName,
        provider,
      },
    ]);

    setModelName("");
  }

  function deleteModel(modelId: number) {
    const model = models.find((item) => item.id === modelId);

    if (!model) {
      return;
    }

    const shouldDelete = window.confirm(
      `Delete model "${model.name}" from the catalog?`,
    );

    if (!shouldDelete) {
      return;
    }

    setModels((currentModels) =>
      currentModels.filter((item) => item.id !== modelId),
    );
  }

  function addDiscoveredOllamaModel(name: string) {
    setModels((currentModels) => {
      if (
        currentModels.some(
          (model) => model.name.toLowerCase() === name.toLowerCase(),
        )
      ) {
        return currentModels;
      }
      return [
        ...currentModels,
        { id: Date.now(), name, provider: "Ollama" },
      ];
    });
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">MODEL CATALOG</span>
          <h1>Models</h1>
          <p className="page-message">
            Manage catalog entries and see which active runtime can execute
            them.
          </p>
        </div>
      </header>

      <section className="panel provider-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">LOCAL LLM AND CODING AGENT</span>
            <h2>Ollama</h2>
            <p className="page-message">
              Local Ollama models can be assigned to agents. Tool-capable models
              run workspace coding tasks through the app’s bounded local agent.
            </p>
          </div>

          <span
            className={`connection-badge ${
              ollamaReady ? "connected" : "disconnected"
            }`}
          >
            {ollamaReady ? "Ready" : "Unavailable"}
          </span>
        </div>

        <div className="provider-connection-grid">
          <div className="provider-actions">
            <button
              className="primary-button"
              disabled={registryBusy}
              onClick={() => void onRefreshRegistry()}
            >
              {registryBusy ? "Checking…" : "Refresh provider registry"}
            </button>
          </div>
        </div>

        {ollamaStatus && (
          <div className="runtime-facts" aria-label="Ollama runtime details">
            <span>
              <strong>Version</strong>
              {ollamaStatus.version ?? "Unavailable"}
            </span>
            <span>
              <strong>Registry adapter</strong>
              {ollamaStatus.provider.displayName}
            </span>
            <span>
              <strong>Installed models</strong>
              {ollamaStatus.models.length}
            </span>
          </div>
        )}

        {ollamaMessage && (
          <div
            className={`runtime-message ${ollamaReady && !registryMessage ? "success" : "error"}`}
            role="status"
          >
            {ollamaMessage}
          </div>
        )}

        {ollamaStatus?.models.length ? (
          <div className="agent-list" style={{ marginTop: "18px" }}>
            {ollamaStatus.models.map((model) => {
              const alreadyRegistered = models.some(
                (item) => item.name.toLowerCase() === model.name.toLowerCase(),
              );
              const modelReady = model.availability === "ready";
              const toolCapable =
                modelReady &&
                model.capabilities.some(
                  (capability) => capability.toLowerCase() === "tools",
                );
              return (
                <article className="agent-card" key={model.name}>
                  <div>
                    <h3>{model.name}</h3>
                    <p>
                      Local Ollama model
                      {!modelReady
                        ? " · metadata unavailable"
                        : toolCapable
                          ? " · coding-agent ready"
                          : " · LLM only"}
                    </p>
                    <small>
                      {model.contextLength
                        ? `${model.contextLength.toLocaleString()} token context`
                        : "Context length unavailable"}
                      {model.contextLength !== null &&
                      model.contextLength < 64_000
                        ? " · keep complex coding tasks focused"
                        : ""}
                    </small>
                    <small>{model.message}</small>
                  </div>

                  <button
                    className={
                      alreadyRegistered ? "secondary-button" : "primary-button"
                    }
                    disabled={alreadyRegistered || !modelReady}
                    onClick={() => addDiscoveredOllamaModel(model.name)}
                  >
                    {alreadyRegistered
                      ? "In catalog"
                      : modelReady
                        ? "Add to catalog"
                        : "Unavailable"}
                  </button>
                </article>
              );
            })}
          </div>
        ) : null}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">ADD MODEL</span>
            <h2>Register a model</h2>
          </div>
        </div>

        <div className="model-composer">
          <label className="form-field">
            <span>Model name</span>
            <input
              type="text"
              value={modelName}
              onChange={(event) => setModelName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  addModel();
                }
              }}
              placeholder="Example: gpt-5"
            />
          </label>

          <label className="form-field">
            <span>Provider</span>
            <select
              value={provider}
              onChange={(event) =>
                setProvider(event.target.value as ModelProvider)
              }
            >
              <option value="OpenAI">OpenAI</option>
              <option value="Anthropic">Anthropic</option>
              <option value="Google">Google</option>
              <option value="Ollama">Ollama</option>
              <option value="Custom">Custom</option>
            </select>
            <small>
              {selectedBinding?.message ??
                "This catalog provider has no registry binding."}
            </small>
          </label>

          <button className="primary-button" onClick={addModel}>
            Add model
          </button>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AVAILABLE MODELS</span>
            <h2>Model catalog</h2>
          </div>
        </div>

        {models.length === 0 ? (
          <p className="page-message">
            No models registered yet. Add your first model above.
          </p>
        ) : (
          <div className="agent-list">
            {models.map((model) => {
              const availability = resolveModelAvailability(
                models,
                model.name,
                providerRegistry,
                activeProvider,
              );
              return (
                <article className="agent-card" key={model.id}>
                  <div>
                    <h3>{model.name}</h3>
                    <p>
                      {model.provider} · {availability.providerId ?? "no adapter"}
                    </p>
                    <small>{availability.reason}</small>
                  </div>

                  <div className="task-card-actions">
                    <span
                      className={`connection-badge ${availability.eligible ? "connected" : "disconnected"}`}
                    >
                      {availability.eligible ? "Executable" : "Unavailable"}
                    </span>
                    <button
                      className="danger-button"
                      onClick={() => deleteModel(model.id)}
                    >
                      Delete
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}
