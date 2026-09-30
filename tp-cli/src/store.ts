import { randomUUID } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join } from "node:path";
import { CommandError, hasErrorCode } from "./errors.js";

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

export function getDataDir(): string {
  return join(homedir(), ".tp");
}

export function getDataFile(dataDir: string = getDataDir()): string {
  return join(dataDir, "bookmarks.json");
}

export function getConfigFile(dataDir: string = getDataDir()): string {
  return join(dataDir, "config.json");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isBookmark(value: unknown): value is Bookmark {
  return (
    isRecord(value) &&
    isNonEmptyString(value.alias) &&
    isNonEmptyString(value.path) &&
    typeof value.createdAt === "number" &&
    Number.isFinite(value.createdAt)
  );
}

function parseJson(raw: string, invalidJsonMessage: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    throw new CommandError(invalidJsonMessage);
  }
}

export function loadConfig(configFile: string): TpConfig {
  if (!existsSync(configFile)) {
    return {};
  }

  const value = parseJson(
    readFileSync(configFile, "utf-8"),
    `Invalid JSON in config file: ${configFile}`,
  );
  if (
    !isRecord(value) ||
    (value.caseSensitive !== undefined &&
      typeof value.caseSensitive !== "boolean")
  ) {
    throw new CommandError(`Invalid config schema: ${configFile}`);
  }
  return { caseSensitive: value.caseSensitive };
}

export function ensureBookmarksFile(dataFile: string): void {
  if (!existsSync(dataFile)) {
    saveBookmarks(dataFile, []);
  }
}

export function loadBookmarks(dataFile: string): Bookmark[] {
  ensureBookmarksFile(dataFile);
  const value = parseJson(
    readFileSync(dataFile, "utf-8"),
    `Invalid JSON in bookmarks file: ${dataFile}`,
  );
  if (!Array.isArray(value) || !value.every(isBookmark)) {
    throw new CommandError(`Invalid bookmarks schema: ${dataFile}`);
  }
  return value;
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

export interface BookmarkUpdate {
  readonly message: string;
  readonly nextBookmarks?: readonly Bookmark[];
}

export function updateBookmarks(
  dataFile: string,
  update: (bookmarks: readonly Bookmark[]) => BookmarkUpdate,
): string {
  return withLock(`${dataFile}.lock`, () => {
    const { message, nextBookmarks } = update(loadBookmarks(dataFile));
    if (nextBookmarks !== undefined) {
      saveBookmarks(dataFile, nextBookmarks);
    }
    return message;
  });
}

function withLock<T>(lockFile: string, operation: () => T): T {
  mkdirSync(dirname(lockFile), { recursive: true });
  if (!acquireLock(lockFile)) {
    throw new CommandError("Bookmarks are busy. Please retry.");
  }
  try {
    return operation();
  } finally {
    rmSync(lockFile, { force: true });
  }
}

function acquireLock(lockFile: string): boolean {
  for (let attempt = 0; attempt < LOCK_RETRY_COUNT; attempt += 1) {
    try {
      writeFileSync(lockFile, String(process.pid), { flag: "wx", mode: 0o600 });
      return true;
    } catch (error) {
      if (!hasErrorCode(error, "EEXIST")) {
        throw error;
      }
    }

    if (isStaleLock(lockFile)) {
      rmSync(lockFile, { force: true });
    } else {
      sleepSync(LOCK_RETRY_DELAY_MS);
    }
  }
  return false;
}

function isStaleLock(lockFile: string): boolean {
  const stats = statSync(lockFile, { throwIfNoEntry: false });
  return stats !== undefined && Date.now() - stats.mtimeMs > STALE_LOCK_AGE_MS;
}

function sleepSync(milliseconds: number): void {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
}
