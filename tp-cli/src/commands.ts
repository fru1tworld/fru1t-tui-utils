import { existsSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { CommandError } from "./errors.js";
import {
  type Bookmark,
  loadBookmarks,
  type TpConfig,
  updateBookmarks,
} from "./store.js";

export const CD_DIRECTIVE_PREFIX = "__TP_CD__:";

export type ListOrder = "utf8" | "recent";

export const SUPPORTED_SHELLS = ["bash", "zsh", "fish", "nu"] as const;
export type Shell = (typeof SUPPORTED_SHELLS)[number];

const ALIAS_COLUMN_WIDTH = 15;
const RESERVED_ALIASES: ReadonlySet<string> = new Set([
  "add",
  "set",
  "del",
  "ch",
  "gc",
  "init",
  "list",
  "help",
  "-h",
  "--help",
  "-v",
  "--version",
  "--completions",
]);

const USAGE = {
  go: "Usage: tp <alias>",
  add: "Usage: tp add <alias>",
  set: "Usage: tp set <alias> <path> [<alias> <path> ...]",
  del: "Usage: tp del <alias>",
  ch: "Usage: tp ch <old_alias> <new_alias>",
  list: "Usage: tp list [-u|--utf8] [-r|--recent]",
  init: `Usage: tp-cli init <${SUPPORTED_SHELLS.join("|")}>`,
} as const;

function validateAlias(
  alias: string | undefined,
  usage: string,
): asserts alias is string {
  if (!alias) {
    throw new CommandError(usage);
  }
  if (/\s/u.test(alias)) {
    throw new CommandError("Aliases cannot contain whitespace.");
  }
  if (RESERVED_ALIASES.has(alias.toLowerCase()) || alias.startsWith("-")) {
    throw new CommandError(`Alias '${alias}' is reserved.`);
  }
}

function aliasesEqual(a: string, b: string, config: TpConfig): boolean {
  return config.caseSensitive ? a === b : a.toLowerCase() === b.toLowerCase();
}

function findAliasIndex(
  bookmarks: readonly Bookmark[],
  alias: string,
  config: TpConfig,
): number {
  return bookmarks.findIndex((bookmark) =>
    aliasesEqual(bookmark.alias, alias, config),
  );
}

function findByAlias(
  bookmarks: readonly Bookmark[],
  alias: string,
  config: TpConfig,
): Bookmark | undefined {
  return bookmarks.find((bookmark) =>
    aliasesEqual(bookmark.alias, alias, config),
  );
}

export function displayPath(path: string, home: string = homedir()): string {
  if (path === home) return "~";
  const prefix = home.endsWith("/") ? home : `${home}/`;
  return path.startsWith(prefix) ? `~/${path.slice(prefix.length)}` : path;
}

function formatBookmarks(bookmarks: readonly Bookmark[]): string {
  return bookmarks
    .map(
      ({ alias, path }) =>
        `  ${alias.padEnd(ALIAS_COLUMN_WIDTH)} -> ${displayPath(path)}`,
    )
    .join("\n");
}

export function add(
  alias: string | undefined,
  cwd: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  validateAlias(alias, USAGE.add);

  return updateBookmarks(dataFile, (bookmarks) => {
    const aliasIndex = findAliasIndex(bookmarks, alias, config);
    const pathOwner = bookmarks.find(
      (bookmark, index) => index !== aliasIndex && bookmark.path === cwd,
    );
    if (pathOwner) {
      throw new CommandError(
        `This path is already registered as '${pathOwner.alias}'.`,
      );
    }

    const existing = bookmarks[aliasIndex];
    if (!existing) {
      return {
        nextBookmarks: [
          { alias, path: cwd, createdAt: Date.now() },
          ...bookmarks,
        ],
        message: `Added: ${alias} -> ${cwd}`,
      };
    }
    if (existing.path === cwd) {
      return { message: `Already registered: ${existing.alias} -> ${cwd}` };
    }
    return {
      nextBookmarks: [
        { ...existing, path: cwd, createdAt: Date.now() },
        ...bookmarks.toSpliced(aliasIndex, 1),
      ],
      message: `Updated: '${existing.alias}' ${existing.path} -> ${cwd}`,
    };
  });
}

interface AliasPath {
  readonly alias: string;
  readonly path: string;
}

function parseAliasPathPairs(
  args: readonly string[],
  cwd: string,
): AliasPath[] {
  if (args.length === 0 || args.length % 2 !== 0) {
    throw new CommandError(USAGE.set);
  }

  const pairs: AliasPath[] = [];
  for (let index = 0; index < args.length; index += 2) {
    const alias = args[index];
    validateAlias(alias, USAGE.set);
    const path = resolve(cwd, args[index + 1]);
    if (!statSync(path, { throwIfNoEntry: false })?.isDirectory()) {
      throw new CommandError(`Directory does not exist: ${path}`);
    }
    pairs.push({ alias, path });
  }
  return pairs;
}

function assertUniqueAliases(
  pairs: readonly AliasPath[],
  config: TpConfig,
): void {
  const repeated = pairs.find((pair, index) =>
    pairs
      .slice(index + 1)
      .some((other) => aliasesEqual(pair.alias, other.alias, config)),
  );
  if (repeated) {
    throw new CommandError(
      `Alias '${repeated.alias}' is specified more than once.`,
    );
  }
}

function assertUniquePaths(bookmarks: readonly Bookmark[]): void {
  for (const [index, bookmark] of bookmarks.entries()) {
    const duplicate = bookmarks
      .slice(index + 1)
      .find((other) => other.path === bookmark.path);
    if (duplicate) {
      throw new CommandError(
        `Path '${bookmark.path}' is assigned to both '${bookmark.alias}' and '${duplicate.alias}'.`,
      );
    }
  }
}

export function set(
  args: readonly string[],
  cwd: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  const pairs = parseAliasPathPairs(args, cwd);
  assertUniqueAliases(pairs, config);

  return updateBookmarks(dataFile, (bookmarks) => {
    const updatedAt = Date.now();
    const setBookmarks = pairs.map(({ alias, path }) => ({
      alias: findByAlias(bookmarks, alias, config)?.alias ?? alias,
      path,
      createdAt: updatedAt,
    }));
    const untouchedBookmarks = bookmarks.filter(
      (bookmark) =>
        !pairs.some(({ alias }) => aliasesEqual(bookmark.alias, alias, config)),
    );
    const nextBookmarks = [...setBookmarks, ...untouchedBookmarks];
    assertUniquePaths(nextBookmarks);

    const noun = pairs.length === 1 ? "bookmark" : "bookmarks";
    return {
      nextBookmarks,
      message: `Set ${pairs.length} ${noun}:\n\n${formatBookmarks(setBookmarks)}`,
    };
  });
}

export function del(
  alias: string | undefined,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!alias) throw new CommandError(USAGE.del);

  return updateBookmarks(dataFile, (bookmarks) => {
    const index = findAliasIndex(bookmarks, alias, config);
    if (index === -1) {
      throw new CommandError(`Alias '${alias}' not found.`);
    }
    return {
      nextBookmarks: bookmarks.toSpliced(index, 1),
      message: `Deleted: ${alias}`,
    };
  });
}

export function gc(dataFile: string): string {
  return updateBookmarks(dataFile, (bookmarks) => {
    const missing = bookmarks.filter(({ path }) => !existsSync(path));
    if (missing.length === 0) {
      return { message: "No invalid bookmarks found. All directories exist." };
    }

    return {
      nextBookmarks: bookmarks.filter(
        (bookmark) => !missing.includes(bookmark),
      ),
      message: [
        `Found ${missing.length} invalid bookmark(s):\n`,
        formatBookmarks(missing),
        `\nRemoved ${missing.length} invalid bookmark(s).`,
      ].join("\n"),
    };
  });
}

export function ch(
  oldAlias: string | undefined,
  newAlias: string | undefined,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!oldAlias) throw new CommandError(USAGE.ch);
  validateAlias(newAlias, USAGE.ch);
  if (aliasesEqual(oldAlias, newAlias, config)) {
    throw new CommandError("Old alias and new alias are the same.");
  }

  return updateBookmarks(dataFile, (bookmarks) => {
    const index = findAliasIndex(bookmarks, oldAlias, config);
    const renamed = bookmarks[index];
    if (!renamed) {
      throw new CommandError(`Alias '${oldAlias}' not found.`);
    }

    const collision = findByAlias(bookmarks, newAlias, config);
    if (!collision) {
      return {
        nextBookmarks: bookmarks.with(index, { ...renamed, alias: newAlias }),
        message: `Renamed: '${oldAlias}' -> '${newAlias}'`,
      };
    }
    if (collision.path !== renamed.path) {
      throw new CommandError(
        `Alias '${newAlias}' already exists with a different path.`,
      );
    }
    return {
      nextBookmarks: bookmarks.toSpliced(index, 1),
      message: [
        `'${oldAlias}' and '${newAlias}' point to the same directory: ${collision.path}`,
        `Removed duplicate alias '${oldAlias}'. Keeping '${newAlias}'.`,
      ].join("\n"),
    };
  });
}

export function go(
  alias: string | undefined,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!alias) {
    throw new CommandError(USAGE.go);
  }

  const bookmark = findByAlias(loadBookmarks(dataFile), alias, config);
  if (!bookmark) {
    throw new CommandError(`Alias '${alias}' not found.`);
  }
  if (!existsSync(bookmark.path)) {
    throw new CommandError(`Directory no longer exists: ${bookmark.path}`);
  }
  return `${CD_DIRECTIVE_PREFIX}${bookmark.path}`;
}

export function parseListOrder(flag: string | undefined): ListOrder {
  switch (flag) {
    case undefined:
    case "-u":
    case "--utf8":
      return "utf8";
    case "-r":
    case "--recent":
      return "recent";
    default:
      throw new CommandError(USAGE.list);
  }
}

function compareUtf8(a: string, b: string): number {
  return Buffer.compare(Buffer.from(a, "utf-8"), Buffer.from(b, "utf-8"));
}

const LIST_ORDER_VIEWS = {
  utf8: {
    header: "UTF-8 order",
    sort: (bookmarks) =>
      bookmarks.toSorted((a, b) => compareUtf8(a.alias, b.alias)),
  },
  recent: {
    header: "newest first",
    sort: (bookmarks) => bookmarks,
  },
} as const satisfies Record<
  ListOrder,
  {
    header: string;
    sort: (bookmarks: readonly Bookmark[]) => readonly Bookmark[];
  }
>;

export function list(dataFile: string, order: ListOrder = "utf8"): string {
  const bookmarks = loadBookmarks(dataFile);
  if (bookmarks.length === 0) {
    return "No bookmarks yet. Use 'tp add <alias>' to add one.";
  }

  const { header, sort } = LIST_ORDER_VIEWS[order];
  return `Bookmarks (${header}):\n\n${formatBookmarks(sort(bookmarks))}`;
}

export function completions(dataFile: string): string {
  return loadBookmarks(dataFile)
    .map(({ alias }) => alias)
    .join("\n");
}

function packageFile(name: string): string {
  return join(dirname(fileURLToPath(import.meta.url)), "..", name);
}

export function version(): string {
  const manifest: { readonly version: string } = JSON.parse(
    readFileSync(packageFile("package.json"), "utf-8"),
  );
  return manifest.version;
}

function isShell(value: string | undefined): value is Shell {
  return SUPPORTED_SHELLS.some((shell) => shell === value);
}

export function shellInit(shell: string | undefined): string {
  if (!isShell(shell)) {
    throw new CommandError(USAGE.init);
  }
  return readFileSync(packageFile(`tp.${shell}`), "utf-8").trimEnd();
}

export function help(): string {
  return `tp - Teleport to bookmarked directories

Usage:
  tp <alias>            Go to bookmarked directory
  tp add <alias>        Add or update current directory bookmark (upsert)
  tp set <alias> <path> [<alias> <path> ...]
                        Set one or more bookmark paths (upsert)
  tp del <alias>        Delete bookmark
  tp ch <old> <new>     Rename alias (or merge if same path)
  tp gc                 Remove bookmarks for non-existent directories
  tp list               Show all bookmarks (UTF-8 order)
  tp list -r            Show all bookmarks (newest first)
  tp help               Show this help
  tp -v, --version      Show version

Shell setup:
  tp-cli init <shell>   Print the shell wrapper (bash|zsh|fish|nu)

  bash  eval "$(tp-cli init bash)"        in ~/.bashrc
  zsh   eval "$(tp-cli init zsh)"         in ~/.zshrc
  fish  tp-cli init fish | source         in ~/.config/fish/config.fish
  nu    tp-cli init nu | save -f ~/.tp/tp.nu   then: source ~/.tp/tp.nu`;
}
