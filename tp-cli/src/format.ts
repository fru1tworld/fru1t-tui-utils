import { homedir } from "node:os";
import { basename, dirname } from "node:path";
import type { Bookmark } from "./store.ts";

export type ListOrder = "utf8" | "recent";

export const LIST_ORDER_HEADERS = {
  utf8: "UTF-8 order",
  recent: "newest first",
} as const satisfies Record<ListOrder, string>;

const ALIAS_COLUMN_WIDTH = 15;

export function displayPath(path: string, home = homedir()): string {
  if (path === home) return "~";
  const prefix = home.endsWith("/") ? home : `${home}/`;
  return path.startsWith(prefix) ? `~/${path.slice(prefix.length)}` : path;
}

// Bookmarks sharing a parent directory are printed under a single parent header
export function formatBookmarks(
  bookmarks: readonly Bookmark[],
  isMissing: (path: string) => boolean = () => false,
): string {
  const groups = Map.groupBy(bookmarks, ({ path }) => dirname(path));
  return [...groups]
    .map(([parent, members]) => {
      const lines = members.map(({ alias, path }) => {
        const mark = isMissing(path) ? "  (missing)" : "";
        return `  ${alias.padEnd(ALIAS_COLUMN_WIDTH)} -> ${basename(path) || path}${mark}`;
      });
      return [displayPath(parent), ...lines].join("\n");
    })
    .join("\n\n");
}

const compareUtf8 = (a: string, b: string): number => Buffer.compare(Buffer.from(a), Buffer.from(b));

export function sortBookmarks(bookmarks: readonly Bookmark[], order: ListOrder): Bookmark[] {
  if (order === "recent") {
    return bookmarks.toSorted((a, b) => b.createdAt - a.createdAt);
  }
  return bookmarks.toSorted(
    (a, b) =>
      compareUtf8(displayPath(dirname(a.path)), displayPath(dirname(b.path))) ||
      compareUtf8(a.alias, b.alias),
  );
}
