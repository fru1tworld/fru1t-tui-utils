import { readFile } from "node:fs/promises";
import { parseArgs } from "node:util";
import { add, type Context, ch, del, gc, go, list, path, set } from "./commands.ts";
import { complete } from "./completion.ts";
import { CommandError, errorCode } from "./errors.ts";
import { shellInit } from "./shell.ts";
import { type CommandName, help, OPTIONS, USAGE } from "./usage.ts";

type Flags = { readonly [Name in keyof typeof OPTIONS]?: boolean };
type Handler = (args: readonly string[], flags: Flags, context: Context) => string | Promise<string>;

const HANDLERS = {
  add: ([alias], _, context) => add(alias, context),
  set: (args, _, context) => set(args, context),
  del: ([alias], _, context) => del(alias, context),
  ch: ([oldAlias, newAlias], _, context) => ch(oldAlias, newAlias, context),
  gc: (_args, _flags, context) => gc(context),
  list: ([filter, ...extra], { recent }, context) => {
    if (extra.length > 0) throw new CommandError(USAGE.list);
    return list({ order: recent ? "recent" : "utf8", filter }, context);
  },
  path: ([target], _, context) => path(target, context),
  help: () => help(),
  init: ([shell]) => shellInit(shell),
} as const satisfies Record<CommandName | "init", Handler>;

export async function version(): Promise<string> {
  const manifest: { readonly version: string } = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf-8"),
  );
  return manifest.version;
}

function parse(argv: readonly string[]) {
  try {
    return parseArgs({ args: [...argv], options: OPTIONS, allowPositionals: true });
  } catch (error) {
    if (!(error instanceof Error && errorCode(error)?.startsWith("ERR_PARSE_ARGS"))) throw error;
    const command = argv[0] ?? "";
    throw new CommandError(Object.hasOwn(USAGE, command) ? USAGE[command as CommandName] : error.message);
  }
}

export async function main(argv: readonly string[], context: Context): Promise<string> {
  // Completion words are passed through verbatim, flags and empty words included
  if (argv[0] === "--complete") return complete(argv.slice(1), context);

  const { values, positionals } = parse(argv);
  if (values.help) return help();
  if (values.version) return version();

  const [command = "list", ...args] = positionals;
  return Object.hasOwn(HANDLERS, command)
    ? HANDLERS[command as keyof typeof HANDLERS](args, values, context)
    : go(command, context);
}
