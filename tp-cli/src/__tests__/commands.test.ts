import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { describe, it } from "node:test";
import { add, CD_DIRECTIVE_PREFIX, ch, del, gc, go, list, path, set } from "../commands.ts";
import { loadBookmarks, saveBookmarks } from "../store.ts";
import { assertIncludes, assertRejects, useSandbox } from "./helpers.ts";

const sandbox = useSandbox();
const at = (cwd: string) => sandbox.context(cwd);
const caseSensitive = (cwd?: string) => sandbox.context(cwd, { caseSensitive: true });
const aliases = async () => (await loadBookmarks(sandbox.dataFile)).map(({ alias }) => alias);

describe("add", () => {
  it("adds a new bookmark", async () => {
    assert.equal(await add("proj", at("/work/proj")), "Added: proj -> /work/proj");
    assert.deepEqual(await aliases(), ["proj"]);
  });

  it("uses the directory name when the alias is omitted", async () => {
    assert.equal(await add(undefined, at("/work/my-app")), "Added: my-app -> /work/my-app");
  });

  it("rejects empty, reserved, and malformed aliases", async () => {
    await assertRejects(add("", at("/x")), "Usage: tp add [alias]");
    await assertRejects(add("list", at("/x")), "Alias 'list' is reserved");
    await assertRejects(add(undefined, at("/work/init")), "Alias 'init' is reserved");
    await assertRejects(add("-x", at("/x")), "is reserved");
    await assertRejects(add("a b", at("/x")), "whitespace or '/'");
    await assertRejects(add("a/b", at("/x")), "whitespace or '/'");
  });

  it("prepends new bookmarks", async () => {
    await add("a", at("/a"));
    await add("b", at("/b"));
    assert.deepEqual(await aliases(), ["b", "a"]);
  });

  it("moves an updated bookmark to the front with a fresh timestamp", async () => {
    await saveBookmarks(sandbox.dataFile, [
      { alias: "other", path: "/other", createdAt: 2 },
      { alias: "study", path: "/old", createdAt: 1 },
    ]);
    assert.equal(await add("study", at("/current")), "Updated: 'study' /old -> /current");
    const [first, second] = await loadBookmarks(sandbox.dataFile);
    assert.equal(first?.path, "/current");
    assert.ok((first?.createdAt ?? 0) > 1);
    assert.equal(second?.alias, "other");
  });

  it("keeps the stored spelling when updating case-insensitively", async () => {
    await add("Proj", at("/a"));
    assert.equal(await add("proj", at("/b")), "Updated: 'Proj' /a -> /b");
    assert.deepEqual(await aliases(), ["Proj"]);
  });

  it("treats aliases differing in case as distinct when caseSensitive", async () => {
    await add("proj", caseSensitive("/a"));
    await add("Proj", caseSensitive("/b"));
    assert.deepEqual(await aliases(), ["Proj", "proj"]);
  });

  it("does not rewrite a bookmark already registered for cwd", async () => {
    const bookmark = { alias: "here", path: "/current", createdAt: 1 };
    await saveBookmarks(sandbox.dataFile, [bookmark]);
    assert.equal(await add("here", at("/current")), "Already registered: here -> /current");
    assert.deepEqual(await loadBookmarks(sandbox.dataFile), [bookmark]);
  });

  it("rejects a path registered under another alias", async () => {
    await add("old", at("/same"));
    await assertRejects(add("new", at("/same")), "already registered as 'old'");
  });
});

describe("set", () => {
  it("sets several bookmarks relative to cwd", async () => {
    await sandbox.mkdirs("first", "second");
    const output = await set(["first", "./first", "second", "second"], sandbox.context());
    assertIncludes(output, "Set 2 bookmarks:");
    assert.deepEqual(
      (await loadBookmarks(sandbox.dataFile)).map(({ alias, path }) => [alias, path]),
      [
        ["first", join(sandbox.dir, "first")],
        ["second", join(sandbox.dir, "second")],
      ],
    );
  });

  it("updates existing aliases and keeps the others", async () => {
    const target = await sandbox.mkdirs("target");
    await add("keep", at("/keep"));
    await add("Move", at("/old"));
    await set(["move", target], sandbox.context());
    assert.deepEqual(
      (await loadBookmarks(sandbox.dataFile)).map(({ alias, path }) => [alias, path]),
      [
        ["Move", target],
        ["keep", "/keep"],
      ],
    );
  });

  it("rejects bad input without changing bookmarks", async () => {
    const dir = await sandbox.mkdirs("dir");
    await add("existing", at(dir));
    await assertRejects(set(["a"], sandbox.context()), "Usage: tp set");
    await assertRejects(set([], sandbox.context()), "Usage: tp set");
    await assertRejects(set(["a", dir, "A", dir], sandbox.context()), "specified more than once");
    await assertRejects(set(["other", dir], sandbox.context()), "assigned to both");
    await assertRejects(set(["a", "./missing"], sandbox.context()), "Directory does not exist");
    assert.deepEqual(await aliases(), ["existing"]);
  });
});

describe("del", () => {
  it("deletes a bookmark case-insensitively by default", async () => {
    await add("Proj", at("/a"));
    assert.equal(await del("proj", sandbox.context()), "Deleted: proj");
    assert.deepEqual(await aliases(), []);
  });

  it("rejects missing and unknown aliases", async () => {
    await add("proj", caseSensitive("/a"));
    await assertRejects(del(undefined, sandbox.context()), "Usage: tp del");
    await assertRejects(del("nope", sandbox.context()), "Alias 'nope' not found.");
    await assertRejects(del("Proj", caseSensitive()), "Alias 'Proj' not found.");
  });
});

describe("gc", () => {
  it("reports when every directory exists", async () => {
    await add("here", sandbox.context());
    assertIncludes(await gc(sandbox.context()), "No invalid bookmarks found");
  });

  it("removes bookmarks whose directory is gone", async () => {
    await add("here", sandbox.context());
    await add("gone", at("/nonexistent/path/xyz"));
    const output = await gc(sandbox.context());
    assertIncludes(output, "Found 1 invalid bookmark(s)");
    assertIncludes(output, "gone");
    assert.deepEqual(await aliases(), ["here"]);
  });
});

describe("ch", () => {
  it("renames an alias case-insensitively by default", async () => {
    await add("Old", at("/a"));
    assert.equal(await ch("old", "new", sandbox.context()), "Renamed: 'old' -> 'new'");
    assert.deepEqual(await aliases(), ["new"]);
  });

  it("rejects missing, identical, and unknown aliases", async () => {
    await add("a", at("/a"));
    await assertRejects(ch(undefined, "b", sandbox.context()), "Usage: tp ch");
    await assertRejects(ch("a", undefined, sandbox.context()), "Usage: tp ch");
    await assertRejects(ch("a", "A", sandbox.context()), "are the same");
    await assertRejects(ch("nope", "b", sandbox.context()), "Alias 'nope' not found.");
  });

  it("rejects renaming onto an alias with a different path", async () => {
    await add("a", at("/a"));
    await add("b", at("/b"));
    await assertRejects(ch("a", "b", sandbox.context()), "already exists with a different path");
  });

  it("merges into an alias pointing to the same path", async () => {
    await saveBookmarks(sandbox.dataFile, [
      { alias: "a", path: "/same", createdAt: 1 },
      { alias: "b", path: "/same", createdAt: 2 },
    ]);
    assertIncludes(await ch("a", "b", sandbox.context()), "Removed duplicate alias 'a'");
    assert.deepEqual(await aliases(), ["b"]);
  });
});

describe("go and path", () => {
  it("resolves an exact alias to the cd directive", async () => {
    await add("here", sandbox.context());
    assert.equal(await go("here", sandbox.context()), `${CD_DIRECTIVE_PREFIX}${sandbox.dir}`);
    assert.equal(await go("HERE", sandbox.context()), `${CD_DIRECTIVE_PREFIX}${sandbox.dir}`);
    await assertRejects(go("HERE", caseSensitive()), "not found");
  });

  it("goes to the only alias matching a prefix and prefers exact matches", async () => {
    await add("payment", at(await sandbox.mkdirs("pay")));
    await add("pas", at(await sandbox.mkdirs("pas")));
    await add("tx", at(await sandbox.mkdirs("tx")));
    assert.equal(await path("t", sandbox.context()), join(sandbox.dir, "tx"));
    await assertRejects(go("pa", sandbox.context()), "Alias 'pa' is ambiguous: pas, payment");
    await add("pa", sandbox.context());
    assert.equal(await path("pa", sandbox.context()), sandbox.dir);
  });

  it("resolves sub directories below a bookmark", async () => {
    await sandbox.mkdirs("app/src/main");
    await add("app", at(join(sandbox.dir, "app")));
    assert.equal(await path("app/src/main", sandbox.context()), join(sandbox.dir, "app/src/main"));
    assert.equal(await path("ap/src", sandbox.context()), join(sandbox.dir, "app/src"));
    await assertRejects(path("app/nope", sandbox.context()), "Directory does not exist");
  });

  it("rejects missing targets and vanished directories", async () => {
    const file = join(sandbox.dir, "file.txt");
    await writeFile(file, "");
    await saveBookmarks(sandbox.dataFile, [
      { alias: "gone", path: "/nonexistent/dir/xyz", createdAt: 1 },
      { alias: "file", path: file, createdAt: 2 },
    ]);
    await assertRejects(go(undefined, sandbox.context()), "Usage: tp <alias>");
    await assertRejects(path(undefined, sandbox.context()), "Usage: tp path");
    await assertRejects(go("nope", sandbox.context()), "Alias 'nope' not found.");
    await assertRejects(go("gone", sandbox.context()), "no longer exists");
    await assertRejects(go("file", sandbox.context()), "no longer exists");
  });
});

describe("list", () => {
  it("explains how to add the first bookmark", async () => {
    assert.equal(await list({}, sandbox.context()), "No bookmarks yet. Use 'tp add <alias>' to add one.");
  });

  it("groups bookmarks under their parent directory in UTF-8 order", async () => {
    for (const dir of ["apps/web", "apps/api", "docs"]) {
      await add(basename(dir), at(await sandbox.mkdirs(dir)));
    }
    assert.equal(
      await list({}, sandbox.context()),
      [
        "Bookmarks (UTF-8 order):",
        "",
        sandbox.dir,
        "  docs            -> docs",
        "",
        join(sandbox.dir, "apps"),
        "  api             -> api",
        "  web             -> web",
      ].join("\n"),
    );
  });

  it("sorts uppercase before lowercase and ASCII before Hangul", async () => {
    await add("Zebra", at("/x/Z"));
    await add("apple", at("/x/a"));
    await add("가나", at("/x/ga"));
    const output = await list({}, sandbox.context());
    assert.ok(output.indexOf("Zebra") < output.indexOf("apple"));
    assert.ok(output.indexOf("apple") < output.indexOf("가나"));
  });

  it("orders newest first by createdAt", async () => {
    await saveBookmarks(sandbox.dataFile, [
      { alias: "old", path: "/a/old", createdAt: 1 },
      { alias: "new", path: "/b/new", createdAt: 2 },
    ]);
    const output = await list({ order: "recent" }, sandbox.context());
    assertIncludes(output, "Bookmarks (newest first):");
    assert.ok(output.indexOf("new") < output.indexOf("old"));
  });

  it("shows paths under the home directory with ~", async () => {
    await add("nested", at(join(homedir(), "dev", "proj")));
    assertIncludes(await list({}, sandbox.context()), "~/dev\n  nested          -> proj");
  });

  it("filters by alias or path, ignoring case", async () => {
    await add("api", at("/work/api"));
    await add("blog", at("/home/blog"));
    const output = await list({ filter: "WORK" }, sandbox.context());
    assertIncludes(output, "api");
    assert.ok(!output.includes("blog"));
    assert.equal(await list({ filter: "zzz" }, sandbox.context()), "No bookmarks match 'zzz'.");
  });

  it("marks bookmarks whose directory is missing", async () => {
    await add("here", sandbox.context());
    await add("gone", at("/nonexistent/dir"));
    const output = await list({}, sandbox.context());
    assertIncludes(output, "gone            -> dir  (missing)");
    assert.doesNotMatch(output, /here .*\(missing\)/);
  });
});
