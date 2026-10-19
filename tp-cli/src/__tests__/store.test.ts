import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, it } from "node:test";
import { StoreError } from "../errors.ts";
import { type Bookmark, loadBookmarks, loadConfig, saveBookmarks, updateBookmarks } from "../store.ts";
import { useSandbox } from "./helpers.ts";

const sandbox = useSandbox();

describe("loadBookmarks", () => {
  it("returns an empty list without creating the file", async () => {
    assert.deepEqual(await loadBookmarks(sandbox.dataFile), []);
    assert.equal(existsSync(sandbox.dataFile), false);
  });

  it("round-trips saved bookmarks", async () => {
    const bookmarks: Bookmark[] = [{ alias: "test", path: "/tmp/test", createdAt: 1 }];
    await saveBookmarks(sandbox.dataFile, bookmarks);
    assert.deepEqual(await loadBookmarks(sandbox.dataFile), bookmarks);
  });

  it("reports malformed JSON as a store error", async () => {
    await writeFile(sandbox.dataFile, "{");
    await assert.rejects(loadBookmarks(sandbox.dataFile), StoreError);
  });

  it("rejects an invalid runtime schema", async () => {
    await writeFile(sandbox.dataFile, JSON.stringify([{ alias: "x", path: 42 }]));
    await assert.rejects(loadBookmarks(sandbox.dataFile), /Invalid bookmarks schema/);
  });
});

describe("updateBookmarks", () => {
  it("serializes concurrent updates", async () => {
    const aliases = ["a", "b", "c", "d", "e"];
    await Promise.all(
      aliases.map((alias) =>
        updateBookmarks(sandbox.dataFile, (bookmarks) => ({
          message: alias,
          nextBookmarks: [...bookmarks, { alias, path: `/${alias}`, createdAt: 1 }],
        })),
      ),
    );
    const saved = (await loadBookmarks(sandbox.dataFile)).map(({ alias }) => alias).toSorted();
    assert.deepEqual(saved, aliases);
  });

  it("leaves the file untouched when nothing changes", async () => {
    assert.equal(await updateBookmarks(sandbox.dataFile, () => ({ message: "noop" })), "noop");
    assert.equal(existsSync(sandbox.dataFile), false);
  });
});

describe("loadConfig", () => {
  const configFile = () => join(sandbox.dir, "config.json");

  it("returns an empty config when the file does not exist", async () => {
    assert.deepEqual(await loadConfig(configFile()), {});
  });

  it("parses a valid config", async () => {
    await writeFile(configFile(), JSON.stringify({ caseSensitive: true }));
    assert.deepEqual(await loadConfig(configFile()), { caseSensitive: true });
  });

  it("rejects invalid JSON", async () => {
    await writeFile(configFile(), "not json");
    await assert.rejects(loadConfig(configFile()), /Invalid JSON in config file/);
  });

  it("rejects an invalid runtime schema", async () => {
    await writeFile(configFile(), JSON.stringify({ caseSensitive: "yes" }));
    await assert.rejects(loadConfig(configFile()), /Invalid config schema/);
  });
});
