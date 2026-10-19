import type { ParseArgsOptionsConfig } from "node:util";

export const SUPPORTED_SHELLS = ["bash", "zsh", "fish", "nu"] as const;
export type Shell = (typeof SUPPORTED_SHELLS)[number];

interface CommandSpec {
  readonly usage: string;
  readonly summary: string;
}

// Single source for dispatch, shell completions, reserved aliases, and help text
const COMMANDS = {
  add: { usage: "tp add [alias]", summary: "Bookmark current directory (alias defaults to its name)" },
  set: {
    usage: "tp set <alias> <path> [<alias> <path> ...]",
    summary: "Set one or more bookmark paths (upsert)",
  },
  del: { usage: "tp del <alias>", summary: "Delete bookmark" },
  ch: { usage: "tp ch <old> <new>", summary: "Rename alias (or merge if same path)" },
  gc: { usage: "tp gc", summary: "Remove bookmarks for non-existent directories" },
  list: { usage: "tp list [-r] [filter]", summary: "Show bookmarks grouped by parent (-r: newest first)" },
  path: { usage: "tp path <alias>[/sub/dir]", summary: "Print bookmark path" },
  help: { usage: "tp help", summary: "Show this help" },
} as const satisfies Record<string, CommandSpec>;

export type CommandName = keyof typeof COMMANDS;
export const COMMAND_NAMES = Object.keys(COMMANDS) as readonly CommandName[];

const LIST_OPTIONS = {
  recent: { type: "boolean", short: "r" },
  utf8: { type: "boolean", short: "u" },
} as const satisfies ParseArgsOptionsConfig;

export const OPTIONS = {
  ...LIST_OPTIONS,
  help: { type: "boolean", short: "h" },
  version: { type: "boolean", short: "v" },
} as const satisfies ParseArgsOptionsConfig;

export const LIST_FLAGS = Object.entries(LIST_OPTIONS).flatMap(([name, { short }]) => [
  `-${short}`,
  `--${name}`,
]);

// Commands only reachable through tp-cli are reserved as well
export const RESERVED_NAMES: ReadonlySet<string> = new Set([...COMMAND_NAMES, "init"]);

export const USAGE = {
  ...(Object.fromEntries(
    Object.entries(COMMANDS).map(([name, { usage }]) => [name, `Usage: ${usage}`]),
  ) as Record<CommandName, string>),
  go: "Usage: tp <alias>[/sub/dir]",
  init: `Usage: tp-cli init <${SUPPORTED_SHELLS.join("|")}>`,
};

const USAGE_COLUMN_WIDTH = 22;

function helpLine(usage: string, summary: string): string {
  return usage.length < USAGE_COLUMN_WIDTH
    ? `  ${usage.padEnd(USAGE_COLUMN_WIDTH)}${summary}`
    : `  ${usage}\n  ${" ".repeat(USAGE_COLUMN_WIDTH)}${summary}`;
}

export function help(): string {
  return [
    "tp - Teleport to bookmarked directories",
    "",
    "Usage:",
    helpLine("tp <alias>[/sub/dir]", "Go to bookmarked directory (unique prefix works)"),
    ...Object.values(COMMANDS).map(({ usage, summary }) => helpLine(usage, summary)),
    helpLine("tp -v, --version", "Show version"),
    "",
    "Shell setup:",
    helpLine("tp-cli init <shell>", `Print the shell wrapper (${SUPPORTED_SHELLS.join("|")})`),
    "",
    '  bash  eval "$(tp-cli init bash)"        in ~/.bashrc',
    '  zsh   eval "$(tp-cli init zsh)"         in ~/.zshrc',
    "  fish  tp-cli init fish | source         in ~/.config/fish/config.fish",
    "  nu    tp-cli init nu | save -f ~/.tp/tp.nu   then: source ~/.tp/tp.nu",
  ].join("\n");
}
