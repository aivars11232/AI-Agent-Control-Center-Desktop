import { describe, expect, it } from "vitest";
import { interpretVoiceCommand } from "./voiceCommand";

const options = {
  openPhrases: ["open", "launch", "start"],
  closePhrases: ["close", "quit", "exit"],
  replacements: "fire fox = firefox\nvisual studio = visual studio code",
};

describe("voice command characterization", () => {
  it("keeps wake-word requests on the coding path", () => {
    expect(interpretVoiceCommand("Lucy refactor the parser", options)).toEqual({
      intent: "coding_request",
      entity: "refactor the parser",
      transcript: "lucy refactor the parser",
    });
  });

  it("strips conversational wording and applies configured replacements", () => {
    expect(
      interpretVoiceCommand(
        "Could you please launch fire fox for me",
        options,
      ),
    ).toEqual({
      intent: "open_application",
      entity: "firefox",
      transcript: "could you please launch fire fox for me",
    });
  });

  it("recognizes close, folder, pointer, desktop, and named-window actions", () => {
    expect(
      interpretVoiceCommand("Would you close visual studio for me", options),
    ).toMatchObject({
      intent: "close_application",
      entity: "visual studio code",
    });
    expect(interpretVoiceCommand("open documents", options)).toMatchObject({
      intent: "open_folder",
      entity: "Documents",
    });
    expect(interpretVoiceCommand("double click", options)).toMatchObject({
      intent: "pointer_action",
      entity: "double-click",
    });
    expect(interpretVoiceCommand("show app launcher", options)).toMatchObject({
      intent: "desktop_action",
      entity: "open-launcher",
    });
    expect(interpretVoiceCommand("maximize firefox", options)).toMatchObject({
      intent: "application_window_action",
      entity: "firefox",
      action: "maximize",
    });
  });

  it("normalizes bounded dictation commands", () => {
    expect(
      interpretVoiceCommand("type first line new line second line", options),
    ).toEqual({
      intent: "text_input",
      entity: "first line\nsecond line",
      transcript: "type first line new line second line",
    });
  });

  it("treats safe bare names as open requests", () => {
    expect(interpretVoiceCommand("firefox", options)).toEqual({
      intent: "open_application",
      entity: "firefox",
      transcript: "firefox",
    });
  });

  it("rejects empty and destructive bare requests", () => {
    expect(interpretVoiceCommand("", options)).toMatchObject({
      intent: "unsupported",
      entity: "",
    });
    expect(interpretVoiceCommand("delete files", options)).toMatchObject({
      intent: "unsupported",
      entity: "",
    });
  });
});
