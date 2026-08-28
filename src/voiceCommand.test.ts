import { describe, expect, it } from "vitest";
import { interpretVoiceCommand } from "./voiceCommand";

const options = {
  openPhrases: ["open", "launch", "start"],
  closePhrases: ["close", "quit", "exit"],
  replacements: "fire fox = firefox\nvisual studio = visual studio code",
};

describe("canonical voice command interpretation", () => {
  it("keeps wake-word requests on the coding-task path", () => {
    expect(interpretVoiceCommand("Lucy refactor the parser", options)).toEqual({
      intent: {
        kind: "createCodingTask",
        request: "refactor the parser",
      },
      transcript: "lucy refactor the parser",
    });
  });

  it("strips conversational wording and resolves known apps to exact desktop IDs", () => {
    expect(
      interpretVoiceCommand(
        "Could you please launch fire fox for me",
        options,
      ),
    ).toEqual({
      intent: {
        kind: "launchApplication",
        application: "firefox.desktop",
      },
      transcript: "could you please launch fire fox for me",
    });
  });

  it("emits canonical folder, pointer, keyboard, and window actions", () => {
    expect(interpretVoiceCommand("open documents", options).intent).toEqual({
      kind: "openStandardFolder",
      folder: "documents",
    });
    expect(interpretVoiceCommand("double click", options).intent).toEqual({
      kind: "pointerAction",
      action: "doubleClick",
    });
    expect(interpretVoiceCommand("show app launcher", options).intent).toEqual({
      kind: "keyboardAction",
      action: "openLauncher",
    });
    expect(interpretVoiceCommand("maximize window", options).intent).toEqual({
      kind: "activeWindowAction",
      action: "maximize",
    });
    expect(interpretVoiceCommand("maximize firefox", options).intent).toEqual({
      kind: "namedWindowAction",
      application: "firefox.desktop",
      action: "maximize",
    });
  });

  it("normalizes bounded dictation into the canonical text intent", () => {
    expect(
      interpretVoiceCommand("type first line new line second line", options),
    ).toEqual({
      intent: {
        kind: "typeText",
        text: "first line\nsecond line",
      },
      transcript: "type first line new line second line",
    });
  });

  it("never turns an unknown named close into an active-window close", () => {
    expect(interpretVoiceCommand("close imaginary editor", options).intent).toEqual({
      kind: "closeApplication",
      application: "imaginary editor",
    });
    expect(interpretVoiceCommand("close active window", options).intent).toEqual({
      kind: "closeActiveWindow",
    });
    expect(interpretVoiceCommand("close", options).intent).toBeNull();
  });

  it("treats safe bare names as launches and rejects unsupported destructive phrases", () => {
    expect(interpretVoiceCommand("firefox", options)).toEqual({
      intent: {
        kind: "launchApplication",
        application: "firefox.desktop",
      },
      transcript: "firefox",
    });
    expect(interpretVoiceCommand("", options).intent).toBeNull();
    expect(interpretVoiceCommand("delete files", options).intent).toBeNull();
  });
});
