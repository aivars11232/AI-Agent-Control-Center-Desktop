import { describe, expect, it } from "vitest";
import { errorMessage } from "./errors";

describe("errorMessage", () => {
  it("uses the message of a real Error", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
  });

  it("passes a plain string through", () => {
    expect(errorMessage("provider unavailable")).toBe("provider unavailable");
  });

  it("unpacks a backend PersistenceError object instead of [object Object]", () => {
    const rejection = {
      code: "QUEUE_HEAD_REQUIRED",
      message: "Only the queue head can enter the execute slot.",
      recoverable: true,
    };
    expect(errorMessage(rejection)).toBe(
      "Only the queue head can enter the execute slot. (QUEUE_HEAD_REQUIRED)",
    );
  });

  it("falls back to the code when a rejection carries no message", () => {
    expect(errorMessage({ code: "RUN_BUSY" })).toBe("RUN_BUSY");
  });

  it("uses the message alone when there is no code", () => {
    expect(errorMessage({ message: "Something failed." })).toBe(
      "Something failed.",
    );
  });

  it("never returns the literal [object Object] for an opaque object", () => {
    expect(errorMessage({ unexpected: true })).not.toBe("[object Object]");
  });

  it("returns a generic sentence for null or undefined", () => {
    expect(errorMessage(null)).toBe("An unexpected error occurred.");
    expect(errorMessage(undefined)).toBe("An unexpected error occurred.");
  });
});
