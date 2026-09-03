import { randomUUID } from "node:crypto";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
  writeSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export interface Bookmark {
  readonly alias: string;
  readonly path: string;
  readonly createdAt: number;
}

export interface TpConfig {
  readonly caseSensitive?: boolean;
}

export type ListOrder = "utf8" | "recent";

const ALIAS_COLUMN_WIDTH = 15;
const BOOKMARK_LOCK_RETRY_COUNT = 40;
const BOOKMARK_LOCK_RETRY_DELAY_MS = 25;
const STALE_LOCK_AGE_MS = 10_000;
const RESERVED_ALIASES = new Set([
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

export class CommandError extends Error {
  override readonly name = "CommandError";
}

export function getDataDir(): string {
  return join(homedir(), ".tp");
}

export function getDataFile(dataDir?: string): string {
  return join(dataDir ?? getDataDir(), "bookmarks.json");
}

export function getConfigFile(dataDir?: string): string {
  return join(dataDir ?? getDataDir(), "config.json");
}

export function loadConfig(configFile: string): TpConfig {
  if (!existsSync(configFile)) {
    return {};
  }

  const raw = readFileSync(configFile, "utf-8");
  try {
    const value: unknown = JSON.parse(raw);
    if (!isRecord(value) || !isOptionalBoolean(value.caseSensitive)) {
      throw new CommandError(`Invalid config schema: ${configFile}`);
    }
    return { caseSensitive: value.caseSensitive };
  } catch (error) {
    if (error instanceof CommandError) {
      throw error;
    }
    throw new CommandError(`Invalid JSON in config file: ${configFile}`);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isOptionalBoolean(value: unknown): value is boolean | undefined {
  return value === undefined || typeof value === "boolean";
}

function isBookmark(value: unknown): value is Bookmark {
  return (
    isRecord(value) &&
    typeof value.alias === "string" &&
    value.alias.length > 0 &&
    typeof value.path === "string" &&
    value.path.length > 0 &&
    typeof value.createdAt === "number" &&
    Number.isFinite(value.createdAt)
  );
}

function parseBookmarks(raw: string, dataFile: string): Bookmark[] {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new CommandError(`Invalid JSON in bookmarks file: ${dataFile}`);
  }
  if (!Array.isArray(value) || !value.every(isBookmark)) {
    throw new CommandError(`Invalid bookmarks schema: ${dataFile}`);
  }
  return value;
}

export function init(dataFile: string): void {
  mkdirSync(dirname(dataFile), { recursive: true });
  if (!existsSync(dataFile)) {
    saveBookmarks(dataFile, []);
  }
}

export function loadBookmarks(dataFile: string): Bookmark[] {
  init(dataFile);
  const raw = readFileSync(dataFile, "utf-8");
  try {
    return parseBookmarks(raw, dataFile);
  } catch (error) {
    if (
      error instanceof CommandError &&
      error.message.startsWith("Invalid bookmarks schema")
    ) {
      throw error;
    }
    throw new CommandError(`Invalid JSON in bookmarks file: ${dataFile}`);
  }
}

export function saveBookmarks(
  dataFile: string,
  bookmarks: readonly Bookmark[],
): void {
  mkdirSync(dirname(dataFile), { recursive: true });
  const temporaryFile = join(
    dirname(dataFile),
    `.${basename(dataFile)}.${process.pid}.${randomUUID()}.tmp`,
  );
  try {
    writeFileSync(temporaryFile, JSON.stringify(bookmarks, null, 2), {
      encoding: "utf-8",
      flag: "wx",
      mode: 0o600,
    });
    renameSync(temporaryFile, dataFile);
  } finally {
    rmSync(temporaryFile, { force: true });
  }
}

interface BookmarkMutation<T> {
  readonly bookmarks?: readonly Bookmark[];
  readonly result: T;
}

function withBookmarkLock<T>(dataFile: string, operation: () => T): T {
  mkdirSync(dirname(dataFile), { recursive: true });
  const lockFile = `${dataFile}.lock`;
  const waitBuffer = new Int32Array(new SharedArrayBuffer(4));
  let lockDescriptor: number | undefined;

  for (let attempt = 0; attempt < BOOKMARK_LOCK_RETRY_COUNT; attempt += 1) {
    try {
      lockDescriptor = openSync(lockFile, "wx", 0o600);
      writeSync(lockDescriptor, String(process.pid));
      break;
    } catch (error) {
      const code = isRecord(error) ? error.code : undefined;
      if (code !== "EEXIST") {
        throw error;
      }
      if (isStaleLock(lockFile)) {
        rmSync(lockFile, { force: true });
        continue;
      }
      Atomics.wait(waitBuffer, 0, 0, BOOKMARK_LOCK_RETRY_DELAY_MS);
    }
  }

  if (lockDescriptor === undefined) {
    throw new CommandError("Bookmarks are busy. Please retry.");
  }

  try {
    return operation();
  } finally {
    closeSync(lockDescriptor);
    rmSync(lockFile, { force: true });
  }
}

function isStaleLock(lockFile: string): boolean {
  try {
    return Date.now() - statSync(lockFile).mtimeMs > STALE_LOCK_AGE_MS;
  } catch (error) {
    const code = isRecord(error) ? error.code : undefined;
    if (code === "ENOENT") return false;
    throw error;
  }
}

function mutateBookmarks<T>(
  dataFile: string,
  mutation: (bookmarks: readonly Bookmark[]) => BookmarkMutation<T>,
): T {
  return withBookmarkLock(dataFile, () => {
    const currentBookmarks = loadBookmarks(dataFile);
    const change = mutation(currentBookmarks);
    if (change.bookmarks !== undefined) {
      saveBookmarks(dataFile, change.bookmarks);
    }
    return change.result;
  });
}

function validateAlias(alias: string, usage: string): void {
  if (!alias) {
    throw new CommandError(usage);
  }
  if (alias !== alias.trim() || /\s/u.test(alias)) {
    throw new CommandError("Aliases cannot contain whitespace.");
  }
  if (RESERVED_ALIASES.has(alias.toLowerCase()) || alias.startsWith("-")) {
    throw new CommandError(`Alias '${alias}' is reserved.`);
  }
}

function aliasMatch(a: string, b: string, caseSensitive: boolean): boolean {
  return caseSensitive ? a === b : a.toLowerCase() === b.toLowerCase();
}

function findByAlias(
  bookmarks: readonly Bookmark[],
  alias: string,
  caseSensitive: boolean,
): Bookmark | undefined {
  return bookmarks.find((b) => aliasMatch(b.alias, alias, caseSensitive));
}

function findIndexByAlias(
  bookmarks: readonly Bookmark[],
  alias: string,
  caseSensitive: boolean,
): number {
  return bookmarks.findIndex((b) => aliasMatch(b.alias, alias, caseSensitive));
}

function isCaseSensitive(config: TpConfig): boolean {
  return config.caseSensitive ?? false;
}

export function displayPath(path: string, home: string = homedir()): string {
  if (path === home) return "~";
  const prefix = home.endsWith("/") ? home : `${home}/`;
  return path.startsWith(prefix) ? `~/${path.slice(prefix.length)}` : path;
}

function formatBookmarks(bookmarks: readonly Bookmark[]): string {
  return bookmarks
    .map(
      (b) =>
        `  ${b.alias.padEnd(ALIAS_COLUMN_WIDTH)} -> ${displayPath(b.path)}`,
    )
    .join("\n");
}

export function add(
  alias: string,
  cwd: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  validateAlias(alias, "Usage: tp add <alias>");

  return mutateBookmarks(dataFile, (bookmarks) => {
    const aliasIndex = findIndexByAlias(
      bookmarks,
      alias,
      isCaseSensitive(config),
    );
    const existingAlias = bookmarks[aliasIndex];
    const existingPath = bookmarks.find(
      (bookmark, index) => index !== aliasIndex && bookmark.path === cwd,
    );

    if (existingPath) {
      throw new CommandError(
        `This path is already registered as '${existingPath.alias}'.`,
      );
    }

    if (existingAlias) {
      if (existingAlias.path === cwd) {
        return {
          result: `Already registered: ${existingAlias.alias} -> ${cwd}`,
        };
      }

      return {
        bookmarks: [
          { ...existingAlias, path: cwd, createdAt: Date.now() },
          ...bookmarks.toSpliced(aliasIndex, 1),
        ],
        result: `Updated: '${existingAlias.alias}' ${existingAlias.path} -> ${cwd}`,
      };
    }

    return {
      bookmarks: [{ alias, path: cwd, createdAt: Date.now() }, ...bookmarks],
      result: `Added: ${alias} -> ${cwd}`,
    };
  });
}

export function set(
  args: readonly string[],
  cwd: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (args.length === 0 || args.length % 2 !== 0) {
    throw new CommandError("Usage: tp set <alias> <path> [<alias> <path> ...]");
  }

  const caseSensitive = isCaseSensitive(config);
  const entries = Array.from({ length: args.length / 2 }, (_, index) => {
    const alias = args[index * 2];
    const path = resolve(cwd, args[index * 2 + 1]);

    validateAlias(alias, "Usage: tp set <alias> <path> [<alias> <path> ...]");

    if (!existsSync(path) || !statSync(path).isDirectory()) {
      throw new CommandError(`Directory does not exist: ${path}`);
    }

    return { alias, path };
  });

  for (const [index, entry] of entries.entries()) {
    if (
      entries
        .slice(index + 1)
        .some((other) => aliasMatch(entry.alias, other.alias, caseSensitive))
    ) {
      throw new CommandError(
        `Alias '${entry.alias}' is specified more than once.`,
      );
    }
  }

  return mutateBookmarks(dataFile, (bookmarks) => {
    const untouchedBookmarks = bookmarks.filter(
      (bookmark) =>
        !entries.some(({ alias }) =>
          aliasMatch(bookmark.alias, alias, caseSensitive),
        ),
    );
    const updatedAt = Date.now();
    const updatedBookmarks = entries.map(({ alias, path }) => {
      const existing = findByAlias(bookmarks, alias, caseSensitive);
      return { alias: existing?.alias ?? alias, path, createdAt: updatedAt };
    });
    const nextBookmarks = [...updatedBookmarks, ...untouchedBookmarks];

    for (const [index, bookmark] of nextBookmarks.entries()) {
      const duplicate = nextBookmarks
        .slice(index + 1)
        .find((other) => other.path === bookmark.path);
      if (duplicate) {
        throw new CommandError(
          `Path '${bookmark.path}' is assigned to both '${bookmark.alias}' and '${duplicate.alias}'.`,
        );
      }
    }

    const noun = entries.length === 1 ? "bookmark" : "bookmarks";
    return {
      bookmarks: nextBookmarks,
      result: `Set ${entries.length} ${noun}:\n\n${formatBookmarks(updatedBookmarks)}`,
    };
  });
}

export function del(
  alias: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!alias) throw new CommandError("Usage: tp del <alias>");

  return mutateBookmarks(dataFile, (bookmarks) => {
    const index = findIndexByAlias(bookmarks, alias, isCaseSensitive(config));
    if (index === -1) {
      throw new CommandError(`Alias '${alias}' not found.`);
    }
    return {
      bookmarks: bookmarks.toSpliced(index, 1),
      result: `Deleted: ${alias}`,
    };
  });
}

export function gc(dataFile: string): string {
  return mutateBookmarks(dataFile, (bookmarks) => {
    const existingBookmarks: Bookmark[] = [];
    const missingBookmarks: Bookmark[] = [];

    for (const bookmark of bookmarks) {
      (existsSync(bookmark.path) ? existingBookmarks : missingBookmarks).push(
        bookmark,
      );
    }

    if (missingBookmarks.length === 0) {
      return { result: "No invalid bookmarks found. All directories exist." };
    }

    return {
      bookmarks: existingBookmarks,
      result: [
        `Found ${missingBookmarks.length} invalid bookmark(s):\n`,
        formatBookmarks(missingBookmarks),
        `\nRemoved ${missingBookmarks.length} invalid bookmark(s).`,
      ].join("\n"),
    };
  });
}

export function ch(
  oldAlias: string,
  newAlias: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!oldAlias || !newAlias)
    throw new CommandError("Usage: tp ch <old_alias> <new_alias>");
  validateAlias(newAlias, "Usage: tp ch <old_alias> <new_alias>");

  const caseSensitive = isCaseSensitive(config);

  if (aliasMatch(oldAlias, newAlias, caseSensitive)) {
    throw new CommandError("Old alias and new alias are the same.");
  }

  return mutateBookmarks(dataFile, (bookmarks) => {
    const index = findIndexByAlias(bookmarks, oldAlias, caseSensitive);
    if (index === -1) {
      throw new CommandError(`Alias '${oldAlias}' not found.`);
    }

    const targetBookmark = bookmarks[index];
    const existingNewAlias = findByAlias(bookmarks, newAlias, caseSensitive);
    if (existingNewAlias) {
      if (existingNewAlias.path !== targetBookmark.path) {
        throw new CommandError(
          `Alias '${newAlias}' already exists with a different path.`,
        );
      }

      return {
        bookmarks: bookmarks.toSpliced(index, 1),
        result: [
          `'${oldAlias}' and '${newAlias}' point to the same directory: ${existingNewAlias.path}`,
          `Removed duplicate alias '${oldAlias}'. Keeping '${newAlias}'.`,
        ].join("\n"),
      };
    }

    return {
      bookmarks: bookmarks.with(index, {
        ...targetBookmark,
        alias: newAlias,
      }),
      result: `Renamed: '${oldAlias}' -> '${newAlias}'`,
    };
  });
}

export function go(
  alias: string,
  dataFile: string,
  config: TpConfig = {},
): string {
  if (!alias) {
    throw new CommandError("Usage: tp <alias>");
  }

  const bookmarks = loadBookmarks(dataFile);
  const bookmark = findByAlias(bookmarks, alias, isCaseSensitive(config));

  if (!bookmark) {
    throw new CommandError(`Alias '${alias}' not found.`);
  }

  if (!existsSync(bookmark.path)) {
    throw new CommandError(`Directory no longer exists: ${bookmark.path}`);
  }

  return `__TP_CD__:${bookmark.path}`;
}

export function parseListOrder(arg?: string): ListOrder {
  switch (arg) {
    case undefined:
    case "-u":
    case "--utf8":
      return "utf8";
    case "-r":
    case "--recent":
      return "recent";
    default:
      throw new CommandError("Usage: tp list [-u|--utf8] [-r|--recent]");
  }
}

function compareUtf8(a: string, b: string): number {
  return Buffer.from(a, "utf-8").compare(Buffer.from(b, "utf-8"));
}

const LIST_ORDERS = {
  utf8: {
    header: "UTF-8 order",
    sort: (bookmarks: readonly Bookmark[]) =>
      bookmarks.toSorted((a, b) => compareUtf8(a.alias, b.alias)),
  },
  recent: {
    header: "newest first",
    sort: (bookmarks: readonly Bookmark[]) => bookmarks,
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

  const { header, sort } = LIST_ORDERS[order];
  return `Bookmarks (${header}):\n\n${formatBookmarks(sort(bookmarks))}`;
}

function packageRoot(): string {
  return join(dirname(fileURLToPath(import.meta.url)), "..");
}

export function version(): string {
  const pkgPath = join(packageRoot(), "package.json");
  const pkg = JSON.parse(readFileSync(pkgPath, "utf-8")) as { version: string };
  return pkg.version;
}

export const SUPPORTED_SHELLS = ["bash", "zsh", "fish", "nu"] as const;

export type Shell = (typeof SUPPORTED_SHELLS)[number];

function isShell(value: string | undefined): value is Shell {
  return SUPPORTED_SHELLS.some((shell) => shell === value);
}

export function shellInit(shell?: string): string {
  if (!isShell(shell)) {
    throw new CommandError(
      `Usage: tp-cli init <${SUPPORTED_SHELLS.join("|")}>`,
    );
  }

  return readFileSync(join(packageRoot(), `tp.${shell}`), "utf-8").trimEnd();
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

export function completions(dataFile: string): string {
  return loadBookmarks(dataFile)
    .map((b) => b.alias)
    .join("\n");
}
