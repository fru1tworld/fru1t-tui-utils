import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { execa } from "execa";
import { main, version } from "../cli.ts";
import { CD_DIRECTIVE_PREFIX } from "../commands.ts";
import { assertIncludes, assertRejects, useSandbox } from "./helpers.ts";

const sandbox = useSandbox();
const run = (...argv: string[]) => main(argv, sandbox.context());
const entry = fileURLToPath(new URL("../index.ts", import.meta.url));

describe("main", () => {
  it("shows help and version", async () => {
    const manifest = JSON.parse(await readFile(new URL("../../package.json", import.meta.url), "utf-8"));
    for (const flag of ["help", "-h", "--help"]) {
      assertIncludes(await run(flag), "tp - Teleport to bookmarked directories");
    }
    for (const flag of ["-v", "--version"]) {
      assert.equal(await run(flag), manifest.version);
    }
    assert.equal(await version(), manifest.version);
  });

  it("lists bookmarks by default and with flags or a filter", async () => {
    assertIncludes(await run(), "No bookmarks yet");
    await run("add", "zulu");
    assertIncludes(await run("list", "-r"), "Bookmarks (newest first):");
    assertIncludes(await run("-r"), "Bookmarks (newest first):");
    assert.equal(await run("list", "nomatch"), "No bookmarks match 'nomatch'.");
  });

  it("rejects unknown flags and extra arguments with the command usage", async () => {
    await assertRejects(run("list", "--nope"), "Usage: tp list");
    await assertRejects(run("list", "a", "b"), "Usage: tp list");
    await assertRejects(run("--recent=yes"), "--recent");
  });

  it("routes every bookmark command", async () => {
    await sandbox.mkdirs("first");
    assert.equal(await run("add", "here"), `Added: here -> ${sandbox.dir}`);
    assertIncludes(await run("set", "first", "./first"), "Set 1 bookmark:");
    assert.equal(await run("ch", "first", "second"), "Renamed: 'first' -> 'second'");
    assert.equal(await run("path", "here/first"), join(sandbox.dir, "first"));
    assert.equal(await run("del", "second"), "Deleted: second");
    assertIncludes(await run("gc"), "No invalid bookmarks");
  });

  it("goes to an alias given as the command", async () => {
    await run("add", "here");
    assert.equal(await run("here"), `${CD_DIRECTIVE_PREFIX}${sandbox.dir}`);
    await assertRejects(run("nonexistent"), "Alias 'nonexistent' not found.");
  });

  it("prints shell wrappers", async () => {
    assertIncludes(await run("init", "zsh"), "compdef _tp_completions_zsh tp");
    await assertRejects(run("init"), "Usage: tp-cli init");
  });

  it("passes completion words through verbatim", async () => {
    await sandbox.mkdirs("sub");
    await run("add", "here");
    assert.equal(await run("--complete", "here/"), "here/sub/");
    assert.equal(await run("--complete", "list", "--r"), "--recent");
  });
});

describe("tp-cli process", () => {
  const tpCli = (...args: string[]) =>
    execa(process.execPath, [entry, ...args], {
      cwd: sandbox.dir,
      env: { HOME: sandbox.dir, USERPROFILE: sandbox.dir },
      reject: false,
    });

  it("prints results on stdout", async () => {
    const { exitCode, stdout } = await tpCli("add", "mydir");
    assert.equal(exitCode, 0);
    assert.equal(stdout, `Added: mydir -> ${sandbox.dir}`);
  });

  it("prints errors on stderr and exits with status 1", async () => {
    const { exitCode, stdout, stderr } = await tpCli("nonexistent");
    assert.deepEqual(
      { exitCode, stdout, stderr },
      { exitCode: 1, stdout: "", stderr: "Alias 'nonexistent' not found." },
    );
  });

  it("does not create the bookmarks file when only listing", async () => {
    await tpCli("list");
    assert.equal(existsSync(join(sandbox.dir, ".tp", "bookmarks.json")), false);
  });
});
