import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { hasErrorCode, StoreError } from "./errors.ts";

export interface Bookmark {
  readonly alias: string;
  readonly path: string;
  readonly createdAt: number;
}

export interface TpConfig {
  readonly caseSensitive?: boolean;
}

const LOCK_RETRY_COUNT = 40;
const LOCK_RETRY_DELAY_MS = 25;
const STALE_LOCK_AGE_MS = 10_000;

const getDataDir = (): string => join(homedir(), ".tp");
export const getDataFile = (dataDir = getDataDir()): string => join(dataDir, "bookmarks.json");
export const getConfigFile = (dataDir = getDataDir()): string => join(dataDir, "config.json");

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isNonEmptyString = (value: unknown): value is string => typeof value === "string" && value.length > 0;

const isBookmark = (value: unknown): value is Bookmark =>
  isRecord(value) &&
  isNonEmptyString(value.alias) &&
  isNonEmptyString(value.path) &&
  typeof value.createdAt === "number" &&
  Number.isFinite(value.createdAt);

// Returns undefined for a missing file so that reads never create it
async function readOptionalFile(file: string): Promise<string | undefined> {
  try {
    return await readFile(file, "utf-8");
  } catch (error) {
    if (hasErrorCode(error, "ENOENT")) return undefined;
    throw error;
  }
}

function parseJson(raw: string, invalidJsonMessage: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    throw new StoreError(invalidJsonMessage);
  }
}

export async function loadConfig(configFile: string): Promise<TpConfig> {
  const raw = await readOptionalFile(configFile);
  if (raw === undefined) return {};

  const value = parseJson(raw, `Invalid JSON in config file: ${configFile}`);
  if (!isRecord(value) || (value.caseSensitive !== undefined && typeof value.caseSensitive !== "boolean")) {
    throw new StoreError(`Invalid config schema: ${configFile}`);
  }
  return { caseSensitive: value.caseSensitive };
}

export async function loadBookmarks(dataFile: string): Promise<Bookmark[]> {
  const raw = await readOptionalFile(dataFile);
  if (raw === undefined) return [];

  const value = parseJson(raw, `Invalid JSON in bookmarks file: ${dataFile}`);
  if (!Array.isArray(value) || !value.every(isBookmark)) {
    throw new StoreError(`Invalid bookmarks schema: ${dataFile}`);
  }
  return value;
}

export async function saveBookmarks(dataFile: string, bookmarks: readonly Bookmark[]): Promise<void> {
  await mkdir(dirname(dataFile), { recursive: true });
  const temporaryFile = join(dirname(dataFile), `.${basename(dataFile)}.${process.pid}.${randomUUID()}.tmp`);
  try {
    await writeFile(temporaryFile, JSON.stringify(bookmarks, null, 2), { flag: "wx", mode: 0o600 });
    await rename(temporaryFile, dataFile);
  } finally {
    await rm(temporaryFile, { force: true });
  }
}

export interface BookmarkUpdate {
  readonly message: string;
  readonly nextBookmarks?: readonly Bookmark[];
}

export async function updateBookmarks(
  dataFile: string,
  update: (bookmarks: readonly Bookmark[]) => BookmarkUpdate | Promise<BookmarkUpdate>,
): Promise<string> {
  return withLock(`${dataFile}.lock`, async () => {
    const { message, nextBookmarks } = await update(await loadBookmarks(dataFile));
    if (nextBookmarks !== undefined) {
      await saveBookmarks(dataFile, nextBookmarks);
    }
    return message;
  });
}

async function withLock<T>(lockFile: string, operation: () => Promise<T>): Promise<T> {
  await mkdir(dirname(lockFile), { recursive: true });
  if (!(await acquireLock(lockFile))) {
    throw new StoreError("Bookmarks are busy. Please retry.");
  }
  try {
    return await operation();
  } finally {
    await rm(lockFile, { force: true });
  }
}

async function acquireLock(lockFile: string): Promise<boolean> {
  for (let attempt = 0; attempt < LOCK_RETRY_COUNT; attempt += 1) {
    try {
      await writeFile(lockFile, String(process.pid), { flag: "wx", mode: 0o600 });
      return true;
    } catch (error) {
      if (!hasErrorCode(error, "EEXIST")) throw error;
    }

    if (await isStaleLock(lockFile)) {
      await rm(lockFile, { force: true });
    } else {
      await sleep(LOCK_RETRY_DELAY_MS);
    }
  }
  return false;
}

async function isStaleLock(lockFile: string): Promise<boolean> {
  const stats = await stat(lockFile).catch(() => undefined);
  return stats !== undefined && Date.now() - stats.mtimeMs > STALE_LOCK_AGE_MS;
}
