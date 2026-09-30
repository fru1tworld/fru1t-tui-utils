import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  add,
  ch,
  completions,
  del,
  gc,
  go,
  list,
  SUPPORTED_SHELLS,
  set,
  shellInit,
} from "../commands.js";
import { CommandError } from "../errors.js";
import {
  type Bookmark,
  loadBookmarks,
  saveBookmarks,
  type TpConfig,
} from "../store.js";

let tmpDir: string;
let dataFile: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tp-test-"));
  dataFile = path.join(tmpDir, "bookmarks.json");
});

afterEach(() => {
  vi.unstubAllEnvs();
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

describe("add", () => {
  it("adds a new bookmark", () => {
    const result = add("proj", "/home/user/proj", dataFile);
    expect(result).toBe("Added: proj -> /home/user/proj");
    const bookmarks = loadBookmarks(dataFile);
    expect(bookmarks).toHaveLength(1);
    expect(bookmarks[0].alias).toBe("proj");
    expect(bookmarks[0].path).toBe("/home/user/proj");
  });

  it("throws on missing alias", () => {
    expect(() => add("", "/tmp", dataFile)).toThrow(CommandError);
    expect(() => add("", "/tmp", dataFile)).toThrow("Usage: tp add <alias>");
  });

  it("rejects reserved command names", () => {
    expect(() => add("list", "/tmp", dataFile)).toThrow(
      "Alias 'list' is reserved",
    );
  });

  it("updates an existing alias", () => {
    add("dup", "/a", dataFile);
    const result = add("dup", "/b", dataFile);
    expect(result).toBe("Updated: 'dup' /a -> /b");
    expect(loadBookmarks(dataFile)).toHaveLength(1);
    expect(loadBookmarks(dataFile)[0].path).toBe("/b");
  });

  it("updates case-insensitive aliases while preserving their name", () => {
    add("Work", "/a", dataFile);
    expect(add("work", "/b", dataFile)).toBe("Updated: 'Work' /a -> /b");
    expect(loadBookmarks(dataFile)[0]).toMatchObject({
      alias: "Work",
      path: "/b",
    });
  });

  it("allows case-different alias when caseSensitive is true", () => {
    const config: TpConfig = { caseSensitive: true };
    add("Work", "/a", dataFile, config);
    const result = add("work", "/b", dataFile, config);
    expect(result).toBe("Added: work -> /b");
  });

  it("throws on duplicate path", () => {
    add("first", "/same", dataFile);
    expect(() => add("second", "/same", dataFile)).toThrow(CommandError);
    expect(() => add("second", "/same", dataFile)).toThrow(
      "already registered",
    );
  });

  it("prepends new bookmarks (newest first)", () => {
    add("a", "/a", dataFile);
    add("b", "/b", dataFile);
    const bookmarks = loadBookmarks(dataFile);
    expect(bookmarks[0].alias).toBe("b");
    expect(bookmarks[1].alias).toBe("a");
  });

  it("moves an updated bookmark to the front", () => {
    saveBookmarks(dataFile, [
      { alias: "other", path: "/other", createdAt: 2 },
      { alias: "study", path: "/old", createdAt: 1 },
    ]);

    const result = add("study", "/current", dataFile);

    expect(result).toBe("Updated: 'study' /old -> /current");
    const bookmarks = loadBookmarks(dataFile);
    expect(bookmarks[0]).toMatchObject({ alias: "study", path: "/current" });
    expect(bookmarks[0].createdAt).toBeGreaterThan(1);
    expect(bookmarks[1].alias).toBe("other");
  });

  it("does not rewrite a bookmark already registered for cwd", () => {
    const bookmark = { alias: "here", path: "/current", createdAt: 1 };
    saveBookmarks(dataFile, [bookmark]);
    expect(add("here", "/current", dataFile)).toBe(
      "Already registered: here -> /current",
    );
    expect(loadBookmarks(dataFile)).toEqual([bookmark]);
  });

  it("rejects a path registered under another alias", () => {
    add("old", "/old", dataFile);
    add("current", "/current", dataFile);

    expect(() => add("old", "/current", dataFile)).toThrow(
      "already registered as 'current'",
    );
    expect(loadBookmarks(dataFile).find((b) => b.alias === "old")?.path).toBe(
      "/old",
    );
  });
});

describe("set", () => {
  it("sets multiple bookmark paths relative to cwd", () => {
    const todoDir = path.join(tmpDir, "todo-tui");
    const tilDir = path.join(tmpDir, "til-tui");
    fs.mkdirSync(todoDir);
    fs.mkdirSync(tilDir);

    const result = set(
      ["todo", "./todo-tui", "til", "./til-tui"],
      tmpDir,
      dataFile,
    );

    expect(result).toContain("Set 2 bookmarks:");
    expect(loadBookmarks(dataFile)).toMatchObject([
      { alias: "todo", path: todoDir },
      { alias: "til", path: tilDir },
    ]);
  });

  it("updates existing aliases and adds new aliases", () => {
    const firstDir = path.join(tmpDir, "first");
    const secondDir = path.join(tmpDir, "second");
    fs.mkdirSync(firstDir);
    fs.mkdirSync(secondDir);
    add("Existing", tmpDir, dataFile);

    set(["existing", firstDir, "new", secondDir], tmpDir, dataFile);

    expect(loadBookmarks(dataFile)).toMatchObject([
      { alias: "Existing", path: firstDir },
      { alias: "new", path: secondDir },
    ]);
  });

  it("rejects incomplete alias-path pairs without changing bookmarks", () => {
    add("existing", tmpDir, dataFile);

    expect(() => set(["todo"], tmpDir, dataFile)).toThrow("Usage: tp set");
    expect(loadBookmarks(dataFile)).toMatchObject([
      { alias: "existing", path: tmpDir },
    ]);
  });

  it("rejects duplicate paths without changing bookmarks", () => {
    const targetDir = path.join(tmpDir, "target");
    fs.mkdirSync(targetDir);
    add("existing", tmpDir, dataFile);

    expect(() =>
      set(["a", targetDir, "b", targetDir], tmpDir, dataFile),
    ).toThrow("is assigned to both");
    expect(loadBookmarks(dataFile)).toMatchObject([
      { alias: "existing", path: tmpDir },
    ]);
  });

  it("rejects paths that are not directories", () => {
    expect(() => set(["missing", "./missing"], tmpDir, dataFile)).toThrow(
      "Directory does not exist",
    );
    fs.writeFileSync(path.join(tmpDir, "file.txt"), "");
    expect(() => set(["file", "./file.txt"], tmpDir, dataFile)).toThrow(
      "Directory does not exist",
    );
  });
});

describe("del", () => {
  it("deletes a bookmark", () => {
    add("target", "/target", dataFile);
    const result = del("target", dataFile);
    expect(result).toBe("Deleted: target");
    expect(loadBookmarks(dataFile)).toHaveLength(0);
  });

  it("throws on missing alias", () => {
    expect(() => del("", dataFile)).toThrow(CommandError);
    expect(() => del("", dataFile)).toThrow("Usage: tp del <alias>");
  });

  it("throws on not found", () => {
    expect(() => del("nope", dataFile)).toThrow(CommandError);
    expect(() => del("nope", dataFile)).toThrow("not found");
  });

  it("deletes by case-insensitive alias by default", () => {
    add("Work", "/work", dataFile);
    const result = del("work", dataFile);
    expect(result).toBe("Deleted: work");
    expect(loadBookmarks(dataFile)).toHaveLength(0);
  });

  it("does not match case-different alias when caseSensitive is true", () => {
    const config: TpConfig = { caseSensitive: true };
    add("Work", "/work", dataFile, config);
    expect(() => del("work", dataFile, config)).toThrow("not found");
  });
});

describe("gc", () => {
  it("reports no invalid bookmarks when all valid", () => {
    add("tmp", tmpDir, dataFile);
    const result = gc(dataFile);
    expect(result).toBe("No invalid bookmarks found. All directories exist.");
  });

  it("removes invalid bookmarks", () => {
    const bookmarks: Bookmark[] = [
      { alias: "valid", path: tmpDir, createdAt: 1 },
      { alias: "invalid", path: "/nonexistent/path/xyz", createdAt: 2 },
    ];
    saveBookmarks(dataFile, bookmarks);

    const result = gc(dataFile);
    expect(result).toContain("Found 1 invalid bookmark(s):");
    expect(result).toContain("invalid");
    expect(result).toContain("Removed 1 invalid bookmark(s).");
    expect(loadBookmarks(dataFile)).toHaveLength(1);
    expect(loadBookmarks(dataFile)[0].alias).toBe("valid");
  });
});

describe("ch", () => {
  it("renames an alias", () => {
    add("old", "/old", dataFile);
    const result = ch("old", "new", dataFile);
    expect(result).toBe("Renamed: 'old' -> 'new'");
    const bookmarks = loadBookmarks(dataFile);
    expect(bookmarks[0].alias).toBe("new");
  });

  it("throws on missing old alias param", () => {
    expect(() => ch("", "new", dataFile)).toThrow(CommandError);
    expect(() => ch("", "new", dataFile)).toThrow("Usage: tp ch");
  });

  it("throws on missing new alias param", () => {
    expect(() => ch("old", "", dataFile)).toThrow(CommandError);
    expect(() => ch("old", "", dataFile)).toThrow("Usage: tp ch");
  });

  it("throws when old and new are the same", () => {
    expect(() => ch("same", "same", dataFile)).toThrow(CommandError);
    expect(() => ch("same", "same", dataFile)).toThrow("are the same");
  });

  it("throws when old alias not found", () => {
    expect(() => ch("missing", "new", dataFile)).toThrow(CommandError);
    expect(() => ch("missing", "new", dataFile)).toThrow("not found");
  });

  it("throws when new alias exists with different path", () => {
    add("a", "/a", dataFile);
    add("b", "/b", dataFile);
    expect(() => ch("a", "b", dataFile)).toThrow(CommandError);
    expect(() => ch("a", "b", dataFile)).toThrow(
      "already exists with a different path",
    );
  });

  it("renames by case-insensitive alias by default", () => {
    add("Work", "/work", dataFile);
    const result = ch("work", "project", dataFile);
    expect(result).toBe("Renamed: 'work' -> 'project'");
    expect(loadBookmarks(dataFile)[0].alias).toBe("project");
  });

  it("treats case-different old and new as same by default", () => {
    expect(() => ch("work", "Work", dataFile)).toThrow("are the same");
  });

  it("merges when new alias exists with same path", () => {
    const bookmarks: Bookmark[] = [
      { alias: "a", path: "/same", createdAt: 1 },
      { alias: "b", path: "/same", createdAt: 2 },
    ];
    saveBookmarks(dataFile, bookmarks);

    const result = ch("a", "b", dataFile);
    expect(result).toContain("point to the same directory");
    expect(result).toContain("Removed duplicate alias 'a'");
    expect(result).toContain("Keeping 'b'");
    expect(loadBookmarks(dataFile)).toHaveLength(1);
    expect(loadBookmarks(dataFile)[0].alias).toBe("b");
  });
});

describe("go", () => {
  it("returns __TP_CD__ protocol for valid alias", () => {
    add("here", tmpDir, dataFile);
    const result = go("here", dataFile);
    expect(result).toBe(`__TP_CD__:${tmpDir}`);
  });

  it("throws on missing alias", () => {
    expect(() => go("", dataFile)).toThrow(CommandError);
    expect(() => go("", dataFile)).toThrow("Usage: tp <alias>");
  });

  it("throws when alias not found", () => {
    expect(() => go("nope", dataFile)).toThrow(CommandError);
    expect(() => go("nope", dataFile)).toThrow("not found");
  });

  it("matches case-insensitive alias by default", () => {
    add("rfc", tmpDir, dataFile);
    expect(go("RFC", dataFile)).toBe(`__TP_CD__:${tmpDir}`);
    expect(go("Rfc", dataFile)).toBe(`__TP_CD__:${tmpDir}`);
  });

  it("does not match case-different alias when caseSensitive is true", () => {
    const config: TpConfig = { caseSensitive: true };
    add("rfc", tmpDir, dataFile, config);
    expect(() => go("RFC", dataFile, config)).toThrow("not found");
  });

  it("throws when directory no longer exists", () => {
    const bookmarks: Bookmark[] = [
      { alias: "gone", path: "/nonexistent/dir/xyz", createdAt: 1 },
    ];
    saveBookmarks(dataFile, bookmarks);
    expect(() => go("gone", dataFile)).toThrow(CommandError);
    expect(() => go("gone", dataFile)).toThrow("no longer exists");
  });
});

describe("list", () => {
  it("shows message when no bookmarks", () => {
    const result = list(dataFile);
    expect(result).toBe("No bookmarks yet. Use 'tp add <alias>' to add one.");
  });

  it("lists bookmarks in UTF-8 byte order by default", () => {
    add("beta", "/b", dataFile);
    add("alpha", "/a", dataFile);
    const result = list(dataFile);
    expect(result).toContain("Bookmarks (UTF-8 order):");
    expect(result.indexOf("alpha")).toBeLessThan(result.indexOf("beta"));
  });

  it("sorts uppercase before lowercase and ASCII before Hangul", () => {
    add("Zebra", "/Z", dataFile);
    add("apple", "/a", dataFile);
    add("가나", "/ga", dataFile);
    const result = list(dataFile);
    expect(result.indexOf("Zebra")).toBeLessThan(result.indexOf("apple"));
    expect(result.indexOf("apple")).toBeLessThan(result.indexOf("가나"));
  });

  it("lists bookmarks newest first when order is recent", () => {
    add("alpha", "/a", dataFile);
    add("beta", "/b", dataFile);
    const result = list(dataFile, "recent");
    expect(result).toContain("Bookmarks (newest first):");
    expect(result.indexOf("beta")).toBeLessThan(result.indexOf("alpha"));
  });

  it("shows paths under the home directory with ~", () => {
    const home = os.homedir();
    add("home", home, dataFile);
    add("nested", path.join(home, "dev", "proj"), dataFile);
    add("root", "/opt/x", dataFile);
    const result = list(dataFile);
    expect(result).toContain("-> ~\n");
    expect(result).toContain("-> ~/dev/proj");
    expect(result).toContain("-> /opt/x");
    expect(loadBookmarks(dataFile).map((b) => b.path)).toEqual([
      "/opt/x",
      path.join(home, "dev", "proj"),
      home,
    ]);
  });
});

describe("shellInit", () => {
  it("prints the wrapper for every supported shell", () => {
    for (const shell of SUPPORTED_SHELLS) {
      expect(shellInit(shell)).toContain("tp-cli");
      expect(shellInit(shell)).toContain("__TP_CD__:");
    }
  });

  it("matches the shipped shell file", () => {
    const shipped = fs.readFileSync(
      path.join(fileURLToPath(new URL("../..", import.meta.url)), "tp.zsh"),
      "utf-8",
    );
    expect(shellInit("zsh")).toBe(shipped.trimEnd());
  });

  it("throws when shell is missing", () => {
    expect(() => shellInit(undefined)).toThrow(CommandError);
    expect(() => shellInit(undefined)).toThrow("Usage: tp-cli init");
  });

  it("throws on unsupported shell", () => {
    expect(() => shellInit("powershell")).toThrow(CommandError);
    expect(() => shellInit("powershell")).toThrow("bash|zsh|fish|nu");
  });
});

describe("completions", () => {
  it("returns empty string when no bookmarks", () => {
    const result = completions(dataFile);
    expect(result).toBe("");
  });

  it("returns alias list", () => {
    add("alpha", "/alpha", dataFile);
    add("beta", "/beta", dataFile);
    const result = completions(dataFile);
    expect(result).toBe("beta\nalpha");
  });
});
