import type { ModelDefinition } from "../applicationState";

export const ollamaCodingModelName = "qwen2.5-coder:7b";

export function ollamaCodingModel(id: number): ModelDefinition {
  return {
    id,
    name: ollamaCodingModelName,
    provider: "Ollama",
  };
}
