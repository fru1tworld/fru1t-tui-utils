import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CommandError } from "../errors.js";
import {
  type Bookmark,
  ensureBookmarksFile,
  loadBookmarks,
  loadConfig,
} from "../store.js";

let tmpDir: string;
let dataFile: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tp-store-test-"));
  dataFile = path.join(tmpDir, "bookmarks.json");
});

afterEach(() => {
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

describe("ensureBookmarksFile", () => {
  it("creates directory and file when missing", () => {
    const nested = path.join(tmpDir, "sub", "bookmarks.json");
    ensureBookmarksFile(nested);
    expect(fs.existsSync(path.join(tmpDir, "sub"))).toBe(true);
    expect(fs.existsSync(nested)).toBe(true);
    expect(fs.readFileSync(nested, "utf-8")).toBe("[]");
  });
});

describe("loadBookmarks", () => {
  it("returns empty array for new file", () => {
    expect(loadBookmarks(dataFile)).toEqual([]);
  });

  it("returns bookmarks from existing file", () => {
    const bookmarks: Bookmark[] = [
      { alias: "test", path: "/tmp/test", createdAt: 1 },
    ];
    fs.writeFileSync(dataFile, JSON.stringify(bookmarks));
    expect(loadBookmarks(dataFile)).toEqual(bookmarks);
  });

  it("reports malformed JSON as a command error", () => {
    fs.writeFileSync(dataFile, "{");
    expect(() => loadBookmarks(dataFile)).toThrow(
      "Invalid JSON in bookmarks file",
    );
  });

  it("rejects bookmarks with an invalid runtime schema", () => {
    fs.writeFileSync(dataFile, JSON.stringify([{ alias: "x", path: 42 }]));
    expect(() => loadBookmarks(dataFile)).toThrow("Invalid bookmarks schema");
  });
});

describe("loadConfig", () => {
  it("returns empty object when file does not exist", () => {
    expect(loadConfig(path.join(tmpDir, "nonexistent.json"))).toEqual({});
  });

  it("returns parsed config from file", () => {
    const configFile = path.join(tmpDir, "config.json");
    fs.writeFileSync(configFile, JSON.stringify({ caseSensitive: true }));
    expect(loadConfig(configFile)).toEqual({ caseSensitive: true });
  });

  it("throws for invalid JSON", () => {
    const configFile = path.join(tmpDir, "config.json");
    fs.writeFileSync(configFile, "not json");
    expect(() => loadConfig(configFile)).toThrow(CommandError);
    expect(() => loadConfig(configFile)).toThrow("Invalid JSON in config file");
  });

  it("rejects an invalid runtime schema", () => {
    const configFile = path.join(tmpDir, "config.json");
    fs.writeFileSync(configFile, JSON.stringify({ caseSensitive: "yes" }));
    expect(() => loadConfig(configFile)).toThrow("Invalid config schema");
  });
});
