export type StandardFolder = "home" | "desktop" | "documents" | "downloads";

export type PointerAction =
  | "moveLeft"
  | "moveRight"
  | "moveUp"
  | "moveDown"
  | "click"
  | "doubleClick"
  | "scrollUp"
  | "scrollDown";

export type KeyboardAction =
  | "openLauncher"
  | "volumeUp"
  | "volumeDown"
  | "toggleMute"
  | "nextWindow"
  | "previousWindow"
  | "left"
  | "right"
  | "up"
  | "down"
  | "home"
  | "end"
  | "pageUp"
  | "pageDown"
  | "tab"
  | "shiftTab"
  | "enter"
  | "escape"
  | "backspace"
  | "delete"
  | "selectAll"
  | "copy"
  | "cut"
  | "paste"
  | "undo"
  | "redo";

export type WindowAction =
  | "restore"
  | "minimize"
  | "maximize"
  | "snapLeft"
  | "snapRight";

export type CanonicalVoiceIntent =
  | { kind: "createCodingTask"; request: string }
  | { kind: "launchApplication"; application: string }
  | { kind: "openStandardFolder"; folder: StandardFolder }
  | { kind: "closeApplication"; application: string }
  | { kind: "closeActiveWindow" }
  | { kind: "pointerAction"; action: PointerAction }
  | { kind: "keyboardAction"; action: KeyboardAction }
  | { kind: "activeWindowAction"; action: WindowAction }
  | {
      kind: "namedWindowAction";
      application: string;
      action: WindowAction;
    }
  | { kind: "typeText"; text: string };

export type VoiceCommand = {
  intent: CanonicalVoiceIntent | null;
  transcript: string;
};

type InterpreterOptions = {
  openPhrases: string[];
  closePhrases: string[];
  replacements: string;
};

const applicationAliases: Record<string, string> = {
  firefox: "firefox.desktop",
  dolphin: "org.kde.dolphin.desktop",
  "system settings": "systemsettings.desktop",
  settings: "systemsettings.desktop",
  terminal: "org.kde.konsole.desktop",
  konsole: "org.kde.konsole.desktop",
  code: "code.desktop",
  "visual studio code": "code.desktop",
};

const folderAliases: Record<string, StandardFolder> = {
  downloads: "downloads",
  "download folder": "downloads",
  documents: "documents",
  "document folder": "documents",
  desktop: "desktop",
  home: "home",
  "home folder": "home",
};

const pointerAliases: Record<string, PointerAction> = {
  "move mouse left": "moveLeft",
  "move cursor left": "moveLeft",
  "go left": "moveLeft",
  "move left": "moveLeft",
  "move mouse right": "moveRight",
  "move cursor right": "moveRight",
  "go right": "moveRight",
  "move right": "moveRight",
  "move mouse up": "moveUp",
  "move cursor up": "moveUp",
  "move up": "moveUp",
  "move mouse down": "moveDown",
  "move cursor down": "moveDown",
  "move down": "moveDown",
  "left click": "click",
  "click it": "click",
  "press it": "click",
  click: "click",
  "double click": "doubleClick",
  "double click it": "doubleClick",
  "scroll up": "scrollUp",
  "scroll down": "scrollDown",
};

const keyboardAliases: Record<string, KeyboardAction> = {
  "app launcher": "openLauncher",
  "application launcher": "openLauncher",
  "open launcher": "openLauncher",
  "open app launcher": "openLauncher",
  "open application launcher": "openLauncher",
  "show launcher": "openLauncher",
  "show app launcher": "openLauncher",
  "show application launcher": "openLauncher",
  "volume up": "volumeUp",
  "increase volume": "volumeUp",
  "raise volume": "volumeUp",
  "turn volume up": "volumeUp",
  louder: "volumeUp",
  "volume down": "volumeDown",
  "decrease volume": "volumeDown",
  "lower volume": "volumeDown",
  "turn volume down": "volumeDown",
  quieter: "volumeDown",
  mute: "toggleMute",
  "toggle mute": "toggleMute",
  "next window": "nextWindow",
  "previous window": "previousWindow",
  "switch window": "nextWindow",
  "left arrow": "left",
  "right arrow": "right",
  "up arrow": "up",
  "down arrow": "down",
  home: "home",
  end: "end",
  "page up": "pageUp",
  "page down": "pageDown",
  tab: "tab",
  "shift tab": "shiftTab",
  enter: "enter",
  return: "enter",
  "new line": "enter",
  escape: "escape",
  cancel: "escape",
  backspace: "backspace",
  delete: "delete",
  "select all": "selectAll",
  copy: "copy",
  cut: "cut",
  paste: "paste",
  undo: "undo",
  redo: "redo",
};

const activeWindowAliases: Record<string, WindowAction> = {
  "minimize window": "minimize",
  "minimise window": "minimize",
  "maximize window": "maximize",
  "maximise window": "maximize",
  "restore window": "restore",
  "snap window left": "snapLeft",
  "snap window right": "snapRight",
};

const namedWindowActionPrefixes: Record<string, WindowAction> = {
  minimize: "minimize",
  minimise: "minimize",
  maximize: "maximize",
  maximise: "maximize",
  restore: "restore",
};

const activeClosePhrases = new Set([
  "close active window",
  "close current window",
  "close focused window",
  "dismiss active window",
  "dismiss current window",
  "dismiss focused window",
]);

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
    .replace(
      /^(?:(?:can|could|would|will)\s+you\s+|i\s+(?:want|need)\s+you\s+to\s+|please\s+)+/,
      "",
    )
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

function canonicalApplication(value: string) {
  return applicationAliases[value] ?? value;
}

export function interpretVoiceCommand(
  transcript: string,
  options: InterpreterOptions,
): VoiceCommand {
  const original = normalizedText(transcript);
  const replaced = applyReplacements(original, options.replacements);

  if (replaced.startsWith("lucy ")) {
    const request = replaced.replace(/^lucy\s+/, "").trim();
    return {
      intent: request ? { kind: "createCodingTask", request } : null,
      transcript: original,
    };
  }

  const command = stripConversation(replaced);
  const textPrefix = matchingPrefix(command, ["type", "write", "dictate"]);
  if (textPrefix) {
    const text = normalizeDictation(command.slice(textPrefix.length));
    return {
      intent: text ? { kind: "typeText", text } : null,
      transcript: original,
    };
  }

  if (activeClosePhrases.has(command)) {
    return { intent: { kind: "closeActiveWindow" }, transcript: original };
  }

  if (activeWindowAliases[command]) {
    return {
      intent: {
        kind: "activeWindowAction",
        action: activeWindowAliases[command],
      },
      transcript: original,
    };
  }

  if (keyboardAliases[command]) {
    return {
      intent: { kind: "keyboardAction", action: keyboardAliases[command] },
      transcript: original,
    };
  }

  const namedWindowPrefix = matchingPrefix(
    command,
    Object.keys(namedWindowActionPrefixes),
  );
  if (namedWindowPrefix) {
    const application = command.slice(namedWindowPrefix.length).trim();
    if (application) {
      return {
        intent: {
          kind: "namedWindowAction",
          application: canonicalApplication(application),
          action: namedWindowActionPrefixes[namedWindowPrefix],
        },
        transcript: original,
      };
    }
  }

  const closePrefix = matchingPrefix(command, [
    ...options.closePhrases,
    "shut",
    "shut down",
    "dismiss",
  ]);
  if (closePrefix) {
    const application = command.slice(closePrefix.length).trim();
    return {
      intent: application
        ? {
            kind: "closeApplication",
            application: canonicalApplication(application),
          }
        : null,
      transcript: original,
    };
  }

  const openPrefix = matchingPrefix(command, [
    ...options.openPhrases,
    "run",
    "bring up",
    "show",
    "take me to",
    "go to",
  ]);
  if (openPrefix) {
    const target = command.slice(openPrefix.length).trim();
    if (!target) return { intent: null, transcript: original };
    if (keyboardAliases[`${openPrefix} ${target}`] || keyboardAliases[target]) {
      return {
        intent: {
          kind: "keyboardAction",
          action:
            keyboardAliases[`${openPrefix} ${target}`] ??
            keyboardAliases[target],
        },
        transcript: original,
      };
    }
    return {
      intent: folderAliases[target]
        ? { kind: "openStandardFolder", folder: folderAliases[target] }
        : {
            kind: "launchApplication",
            application: canonicalApplication(target),
          },
      transcript: original,
    };
  }

  if (folderAliases[command]) {
    return {
      intent: { kind: "openStandardFolder", folder: folderAliases[command] },
      transcript: original,
    };
  }

  if (pointerAliases[command]) {
    return {
      intent: { kind: "pointerAction", action: pointerAliases[command] },
      transcript: original,
    };
  }

  if (
    command &&
    !/\b(delete|remove|shutdown|restart|sign out|uninstall)\b/.test(command)
  ) {
    return {
      intent: {
        kind: "launchApplication",
        application: canonicalApplication(command),
      },
      transcript: original,
    };
  }

  return { intent: null, transcript: original };
}
