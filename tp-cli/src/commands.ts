import { basename, join, resolve } from "node:path";
import { findAliasIndex, normalizeAlias, requireAlias, resolveAlias, validateAlias } from "./alias.ts";
import { isDirectory } from "./directories.ts";
import { CommandError } from "./errors.ts";
import { displayPath, formatBookmarks, LIST_ORDER_HEADERS, type ListOrder, sortBookmarks } from "./format.ts";
import { type Bookmark, loadBookmarks, type TpConfig, updateBookmarks } from "./store.ts";
import { USAGE } from "./usage.ts";

export const CD_DIRECTIVE_PREFIX = "__TP_CD__:";

export interface Context {
  readonly cwd: string;
  readonly dataFile: string;
  readonly config: TpConfig;
}

export async function add(alias: string | undefined, { cwd, dataFile, config }: Context): Promise<string> {
  const name = alias ?? basename(cwd);
  validateAlias(name, USAGE.add);

  return updateBookmarks(dataFile, (bookmarks) => {
    const aliasIndex = findAliasIndex(bookmarks, name, config);
    const pathOwner = bookmarks.find((bookmark, index) => index !== aliasIndex && bookmark.path === cwd);
    if (pathOwner) {
      throw new CommandError(`This path is already registered as '${pathOwner.alias}'.`);
    }

    const existing = bookmarks[aliasIndex];
    if (!existing) {
      return {
        nextBookmarks: [{ alias: name, path: cwd, createdAt: Date.now() }, ...bookmarks],
        message: `Added: ${name} -> ${cwd}`,
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

async function parseAliasPathPairs(args: readonly string[], cwd: string): Promise<AliasPath[]> {
  if (args.length === 0) throw new CommandError(USAGE.set);

  const pairs: AliasPath[] = [];
  for (let index = 0; index < args.length; index += 2) {
    const alias = args[index];
    const target = args[index + 1];
    validateAlias(alias, USAGE.set);
    if (target === undefined) throw new CommandError(USAGE.set);

    const path = resolve(cwd, target);
    if (!(await isDirectory(path))) {
      throw new CommandError(`Directory does not exist: ${path}`);
    }
    pairs.push({ alias, path });
  }
  return pairs;
}

function assertUniqueAliases(pairs: readonly AliasPath[], config: TpConfig): void {
  const seen = new Set<string>();
  for (const { alias } of pairs) {
    const key = normalizeAlias(alias, config);
    if (seen.has(key)) {
      throw new CommandError(`Alias '${alias}' is specified more than once.`);
    }
    seen.add(key);
  }
}

function assertUniquePaths(bookmarks: readonly Bookmark[]): void {
  const owners = new Map<string, string>();
  for (const { alias, path } of bookmarks) {
    const owner = owners.get(path);
    if (owner !== undefined) {
      throw new CommandError(`Path '${path}' is assigned to both '${owner}' and '${alias}'.`);
    }
    owners.set(path, alias);
  }
}

export async function set(args: readonly string[], { cwd, dataFile, config }: Context): Promise<string> {
  const pairs = await parseAliasPathPairs(args, cwd);
  assertUniqueAliases(pairs, config);

  return updateBookmarks(dataFile, (bookmarks) => {
    const updatedAt = Date.now();
    const setBookmarks = pairs.map(({ alias, path }) => ({
      alias: bookmarks[findAliasIndex(bookmarks, alias, config)]?.alias ?? alias,
      path,
      createdAt: updatedAt,
    }));
    const setKeys = new Set(pairs.map(({ alias }) => normalizeAlias(alias, config)));
    const untouched = bookmarks.filter(({ alias }) => !setKeys.has(normalizeAlias(alias, config)));
    const nextBookmarks = [...setBookmarks, ...untouched];
    assertUniquePaths(nextBookmarks);

    const noun = pairs.length === 1 ? "bookmark" : "bookmarks";
    return {
      nextBookmarks,
      message: `Set ${pairs.length} ${noun}:\n\n${formatBookmarks(setBookmarks)}`,
    };
  });
}

export async function del(alias: string | undefined, { dataFile, config }: Context): Promise<string> {
  if (!alias) throw new CommandError(USAGE.del);

  return updateBookmarks(dataFile, (bookmarks) => ({
    nextBookmarks: bookmarks.toSpliced(requireAlias(bookmarks, alias, config).index, 1),
    message: `Deleted: ${alias}`,
  }));
}

export async function gc({ dataFile }: Context): Promise<string> {
  return updateBookmarks(dataFile, async (bookmarks) => {
    const exists = await Promise.all(bookmarks.map(({ path }) => isDirectory(path)));
    const byExistence = Map.groupBy(bookmarks, (_, index) => exists[index] ?? false);
    const missing = byExistence.get(false) ?? [];
    if (missing.length === 0) {
      return { message: "No invalid bookmarks found. All directories exist." };
    }

    return {
      nextBookmarks: byExistence.get(true) ?? [],
      message: [
        `Found ${missing.length} invalid bookmark(s):\n`,
        formatBookmarks(missing),
        `\nRemoved ${missing.length} invalid bookmark(s).`,
      ].join("\n"),
    };
  });
}

export async function ch(
  oldAlias: string | undefined,
  newAlias: string | undefined,
  { dataFile, config }: Context,
): Promise<string> {
  if (!oldAlias) throw new CommandError(USAGE.ch);
  validateAlias(newAlias, USAGE.ch);
  if (normalizeAlias(oldAlias, config) === normalizeAlias(newAlias, config)) {
    throw new CommandError("Old alias and new alias are the same.");
  }

  return updateBookmarks(dataFile, (bookmarks) => {
    const { index, bookmark: renamed } = requireAlias(bookmarks, oldAlias, config);
    const collision = bookmarks[findAliasIndex(bookmarks, newAlias, config)];
    if (!collision) {
      return {
        nextBookmarks: bookmarks.with(index, { ...renamed, alias: newAlias }),
        message: `Renamed: '${oldAlias}' -> '${newAlias}'`,
      };
    }
    if (collision.path !== renamed.path) {
      throw new CommandError(`Alias '${newAlias}' already exists with a different path.`);
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

// Resolves "<alias>[/sub/dir]" to an existing directory path
async function resolveTarget(
  target: string | undefined,
  usage: string,
  { dataFile, config }: Context,
): Promise<string> {
  if (!target) throw new CommandError(usage);

  const [query = "", ...segments] = target.split("/");
  const bookmark = resolveAlias(await loadBookmarks(dataFile), query, config);
  if (!(await isDirectory(bookmark.path))) {
    throw new CommandError(`Directory no longer exists: ${bookmark.path}`);
  }

  const path = join(bookmark.path, ...segments);
  if (!(await isDirectory(path))) {
    throw new CommandError(`Directory does not exist: ${path}`);
  }
  return path;
}

export const go = async (target: string | undefined, context: Context): Promise<string> =>
  `${CD_DIRECTIVE_PREFIX}${await resolveTarget(target, USAGE.go, context)}`;

export const path = (target: string | undefined, context: Context): Promise<string> =>
  resolveTarget(target, USAGE.path, context);

export interface ListOptions {
  readonly order?: ListOrder;
  readonly filter?: string | undefined;
}

function matchesFilter({ alias, path }: Bookmark, filter: string): boolean {
  const needle = filter.toLowerCase();
  return alias.toLowerCase().includes(needle) || displayPath(path).toLowerCase().includes(needle);
}

export async function list({ order = "utf8", filter }: ListOptions, { dataFile }: Context): Promise<string> {
  const bookmarks = await loadBookmarks(dataFile);
  if (bookmarks.length === 0) {
    return "No bookmarks yet. Use 'tp add <alias>' to add one.";
  }

  const shown =
    filter === undefined ? bookmarks : bookmarks.filter((bookmark) => matchesFilter(bookmark, filter));
  if (shown.length === 0) return `No bookmarks match '${filter}'.`;

  const exists = await Promise.all(shown.map(({ path }) => isDirectory(path)));
  const missingPaths = new Set(shown.filter((_, index) => !exists[index]).map(({ path }) => path));
  const body = formatBookmarks(sortBookmarks(shown, order), (path) => missingPaths.has(path));
  return `Bookmarks (${LIST_ORDER_HEADERS[order]}):\n\n${body}`;
}
