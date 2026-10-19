import { CommandError } from "./errors.ts";
import type { Bookmark, TpConfig } from "./store.ts";
import { RESERVED_NAMES } from "./usage.ts";

export function validateAlias(alias: string | undefined, usage: string): asserts alias is string {
  if (!alias) throw new CommandError(usage);
  if (/[\s/]/u.test(alias)) {
    throw new CommandError("Aliases cannot contain whitespace or '/'.");
  }
  if (RESERVED_NAMES.has(alias.toLowerCase()) || alias.startsWith("-")) {
    throw new CommandError(`Alias '${alias}' is reserved.`);
  }
}

export const normalizeAlias = (alias: string, { caseSensitive }: TpConfig): string =>
  caseSensitive ? alias : alias.toLowerCase();

const aliasesEqual = (a: string, b: string, config: TpConfig): boolean =>
  normalizeAlias(a, config) === normalizeAlias(b, config);

export const findAliasIndex = (bookmarks: readonly Bookmark[], alias: string, config: TpConfig): number =>
  bookmarks.findIndex((bookmark) => aliasesEqual(bookmark.alias, alias, config));

export function requireAlias(
  bookmarks: readonly Bookmark[],
  alias: string,
  config: TpConfig,
): { readonly index: number; readonly bookmark: Bookmark } {
  const index = findAliasIndex(bookmarks, alias, config);
  const bookmark = bookmarks[index];
  if (!bookmark) throw new CommandError(`Alias '${alias}' not found.`);
  return { index, bookmark };
}

// Falls back to a unique prefix match when no alias matches exactly
export function resolveAlias(bookmarks: readonly Bookmark[], query: string, config: TpConfig): Bookmark {
  const exact = bookmarks[findAliasIndex(bookmarks, query, config)];
  if (exact) return exact;

  const prefix = normalizeAlias(query, config);
  const candidates = bookmarks.filter(({ alias }) => normalizeAlias(alias, config).startsWith(prefix));
  const [match, ...rest] = candidates;
  if (!match) throw new CommandError(`Alias '${query}' not found.`);
  if (rest.length > 0) {
    const names = candidates.map(({ alias }) => alias).toSorted();
    throw new CommandError(`Alias '${query}' is ambiguous: ${names.join(", ")}`);
  }
  return match;
}
