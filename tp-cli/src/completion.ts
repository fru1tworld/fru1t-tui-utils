import { findAliasIndex } from "./alias.ts";
import type { Context } from "./commands.ts";
import { completeDirectories } from "./directories.ts";
import { type Bookmark, loadBookmarks, type TpConfig } from "./store.ts";
import { COMMAND_NAMES, LIST_FLAGS } from "./usage.ts";

async function completeTarget(
  bookmarks: readonly Bookmark[],
  word: string,
  config: TpConfig,
): Promise<string[]> {
  const slash = word.indexOf("/");
  if (slash === -1) return bookmarks.map(({ alias }) => alias);

  const alias = word.slice(0, slash);
  const bookmark = bookmarks[findAliasIndex(bookmarks, alias, config)];
  if (!bookmark) return [];
  const paths = await completeDirectories(bookmark.path, word.slice(slash + 1));
  return paths.map((path) => `${alias}/${path}`);
}

async function candidatesFor(
  words: readonly string[],
  bookmarks: readonly Bookmark[],
  { cwd, config }: Context,
): Promise<readonly string[]> {
  const [command = "", ...args] = words;
  const current = words.at(-1) ?? "";
  const aliases = bookmarks.map(({ alias }) => alias);

  if (args.length === 0) {
    return current.includes("/")
      ? completeTarget(bookmarks, current, config)
      : [...COMMAND_NAMES, ...aliases];
  }

  const position = args.length;
  switch (command) {
    case "add":
    case "del":
    case "ch":
      return position === 1 ? aliases : [];
    case "path":
      return position === 1 ? completeTarget(bookmarks, current, config) : [];
    case "set":
      return position % 2 === 1 ? aliases : completeDirectories(cwd, current);
    case "list":
      return LIST_FLAGS;
    default:
      return [];
  }
}

// Shell completion entry point. words are the tokens after "tp", the last one
// being the word under the cursor (possibly empty). Candidates ending in "/"
// expect more input, so wrappers must not append a space after them.
export async function complete(words: readonly string[], context: Context): Promise<string> {
  const current = words.at(-1) ?? "";
  const candidates = await candidatesFor(words, await loadBookmarks(context.dataFile), context);
  return candidates.filter((candidate) => candidate.startsWith(current)).join("\n");
}
