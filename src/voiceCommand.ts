export type VoiceIntent =
  | "open_application"
  | "close_application"
  | "open_folder"
  | "pointer_action"
  | "desktop_action"
  | "application_window_action"
  | "text_input"
  | "coding_request"
  | "unsupported";

export type VoiceCommand = {
  intent: VoiceIntent;
  entity: string;
  action?: string;
  transcript: string;
};

type InterpreterOptions = {
  openPhrases: string[];
  closePhrases: string[];
  replacements: string;
};

const folderAliases: Record<string, string> = {
  downloads: "Downloads",
  "download folder": "Downloads",
  documents: "Documents",
  "document folder": "Documents",
  desktop: "Desktop",
  home: "Home",
  "home folder": "Home",
};

const pointerAliases: Record<string, string> = {
  "move mouse left": "move-left",
  "move cursor left": "move-left",
  "go left": "move-left",
  "move left": "move-left",
  "move mouse right": "move-right",
  "move cursor right": "move-right",
  "go right": "move-right",
  "move right": "move-right",
  "move mouse up": "move-up",
  "move cursor up": "move-up",
  "move up": "move-up",
  "move mouse down": "move-down",
  "move cursor down": "move-down",
  "move down": "move-down",
  "left click": "click",
  "click it": "click",
  "press it": "click",
  click: "click",
  "double click": "double-click",
  "double click it": "double-click",
  "scroll up": "scroll-up",
  "scroll down": "scroll-down",
};

const desktopActionAliases: Record<string, string> = {
  "app launcher": "open-launcher",
  "application launcher": "open-launcher",
  "open launcher": "open-launcher",
  "open app launcher": "open-launcher",
  "open application launcher": "open-launcher",
  "show launcher": "open-launcher",
  "show app launcher": "open-launcher",
  "show application launcher": "open-launcher",
  "volume up": "volume-up",
  "increase volume": "volume-up",
  "raise volume": "volume-up",
  "turn volume up": "volume-up",
  louder: "volume-up",
  "volume down": "volume-down",
  "decrease volume": "volume-down",
  "lower volume": "volume-down",
  "turn volume down": "volume-down",
  quieter: "volume-down",
  mute: "toggle-mute",
  "toggle mute": "toggle-mute",
  "minimize window": "minimize-window",
  "maximize window": "maximize-window",
  "restore window": "restore-window",
  "next window": "next-window",
  "previous window": "previous-window",
  "switch window": "next-window",
  "snap window left": "snap-left",
  "snap window right": "snap-right",
  "left arrow": "left",
  "right arrow": "right",
  "up arrow": "up",
  "down arrow": "down",
  home: "home",
  end: "end",
  "page up": "page-up",
  "page down": "page-down",
  tab: "tab",
  "shift tab": "shift-tab",
  enter: "enter",
  return: "enter",
  "new line": "enter",
  escape: "escape",
  cancel: "escape",
  backspace: "backspace",
  delete: "delete",
  "select all": "select-all",
  copy: "copy",
  cut: "cut",
  paste: "paste",
  undo: "undo",
  redo: "redo",
};

const focusedWindowActionPrefixes: Record<string, string> = {
  minimize: "minimize-window",
  minimise: "minimize-window",
  maximize: "maximize-window",
  maximise: "maximize-window",
  restore: "restore-window",
};

const namedWindowActionPrefixes: Record<string, string> = {
  minimize: "minimize",
  minimise: "minimize",
  maximize: "maximize",
  maximise: "maximize",
  restore: "restore",
};

function normalizedText(value: string) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function applyReplacements(value: string, replacements: string) {
  const rules = replacements
    .split("\n")
    .map((line) => line.split("=", 2).map((part) => normalizedText(part)))
    .filter(([spoken, canonical]) => spoken && canonical)
    .sort(([left], [right]) => right.length - left.length);

  return rules.reduce(
    (current, [spoken, canonical]) => current.split(spoken).join(canonical),
    value,
  );
}

function stripConversation(value: string) {
  return value
    .replace(/^(?:hey\s+)?lucy\s+/, "")
    .replace(/^(?:(?:can|could|would|will)\s+you\s+|i\s+(?:want|need)\s+you\s+to\s+|please\s+)+/, "")
    .replace(/\s+(?:please|for me)$/, "")
    .trim();
}

function matchingPrefix(value: string, phrases: string[]) {
  return phrases
    .map(normalizedText)
    .filter(Boolean)
    .sort((left, right) => right.length - left.length)
    .find((phrase) => value === phrase || value.startsWith(`${phrase} `));
}

function normalizeDictation(value: string) {
  return value
    .replace(/\bnew line\b/g, "\n")
    .replace(/\bforward slash\b/g, "/")
    .replace(/\bslash\b/g, "/")
    .replace(/\bunderscore\b/g, "_")
    .replace(/\bdash\b/g, "-")
    .replace(/\bhyphen\b/g, "-")
    .replace(/\bdot\b/g, ".")
    .replace(/\bcomma\b/g, ",")
    .replace(/\bcolon\b/g, ":")
    .replace(/\bequals\b/g, "=")
    .replace(/\bplus\b/g, "+")
    .replace(/\bquestion mark\b/g, "?")
    .replace(/[^a-z0-9\s\-./_:,=+?@]/g, " ")
    .replace(/[ \t]+/g, " ")
    .replace(/ *\n */g, "\n")
    .trim();
}

export function interpretVoiceCommand(
  transcript: string,
  options: InterpreterOptions,
): VoiceCommand {
  const original = normalizedText(transcript);
  const replaced = applyReplacements(original, options.replacements);

  if (replaced.startsWith("lucy ")) {
    const request = replaced.replace(/^lucy\s+/, "").trim();
    return request
      ? { intent: "coding_request", entity: request, transcript: original }
      : { intent: "unsupported", entity: "", transcript: original };
  }

  const command = stripConversation(replaced);

  const textPrefix = matchingPrefix(command, ["type", "write", "dictate"]);
  if (textPrefix) {
    const text = normalizeDictation(command.slice(textPrefix.length));
    return text
      ? { intent: "text_input", entity: text, transcript: original }
      : { intent: "unsupported", entity: "", transcript: original };
  }

  if (desktopActionAliases[command]) {
    return {
      intent: "desktop_action",
      entity: desktopActionAliases[command],
      transcript: original,
    };
  }

  const focusedWindowAction = matchingPrefix(
    command,
    Object.keys(focusedWindowActionPrefixes),
  );
  if (focusedWindowAction) {
    const application = command.slice(focusedWindowAction.length).trim();
    if (application) {
      return {
        intent: "application_window_action",
        entity: application,
        action: namedWindowActionPrefixes[focusedWindowAction],
        transcript: original,
      };
    }
    return {
      intent: "desktop_action",
      entity: focusedWindowActionPrefixes[focusedWindowAction],
      transcript: original,
    };
  }

  const closePhrases = [...options.closePhrases, "shut", "shut down", "dismiss"];
  const openPhrases = [...options.openPhrases, "run", "bring up", "show", "take me to", "go to"];
  const closePrefix = matchingPrefix(command, closePhrases);
  if (closePrefix) {
    return {
      intent: "close_application",
      entity: command.slice(closePrefix.length).trim(),
      transcript: original,
    };
  }

  const openPrefix = matchingPrefix(command, openPhrases);
  if (openPrefix) {
    const entity = command.slice(openPrefix.length).trim();
    if (desktopActionAliases[`${openPrefix} ${entity}`] || desktopActionAliases[entity]) {
      return {
        intent: "desktop_action",
        entity: desktopActionAliases[`${openPrefix} ${entity}`] ?? desktopActionAliases[entity],
        transcript: original,
      };
    }
    return folderAliases[entity]
      ? { intent: "open_folder", entity: folderAliases[entity], transcript: original }
      : { intent: "open_application", entity, transcript: original };
  }

  if (folderAliases[command]) {
    return { intent: "open_folder", entity: folderAliases[command], transcript: original };
  }

  if (pointerAliases[command]) {
    return { intent: "pointer_action", entity: pointerAliases[command], transcript: original };
  }

  // A bare name can only mean open; it can never imply close or a destructive action.
  if (command && !/\b(delete|remove|shutdown|restart|sign out|uninstall)\b/.test(command)) {
    return { intent: "open_application", entity: command, transcript: original };
  }

  return { intent: "unsupported", entity: "", transcript: original };
}
