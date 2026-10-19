import { readdir, stat } from "node:fs/promises";
import { join, resolve } from "node:path";

export async function isDirectory(path: string): Promise<boolean> {
  const stats = await stat(path).catch(() => undefined);
  return stats?.isDirectory() ?? false;
}

// Completes a partial relative path under base, e.g. "src/ma" -> ["src/main/"].
// Hidden directories are offered only when the partial name starts with ".".
export async function completeDirectories(base: string, partialPath: string): Promise<string[]> {
  const parent = partialPath.slice(0, partialPath.lastIndexOf("/") + 1);
  const partial = partialPath.slice(parent.length);
  const directory = resolve(base, parent);

  const entries = await readdir(directory).catch(() => []);
  const names = entries.filter(
    (entry) => entry.startsWith(partial) && (partial.startsWith(".") || !entry.startsWith(".")),
  );
  const isDirectoryFlags = await Promise.all(names.map((name) => isDirectory(join(directory, name))));
  return names
    .filter((_, index) => isDirectoryFlags[index])
    .toSorted()
    .map((name) => `${parent}${name}/`);
}
