import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { beforeEach, describe, it } from "node:test";
import { add } from "../commands.ts";
import { complete } from "../completion.ts";
import { COMMAND_NAMES, LIST_FLAGS } from "../usage.ts";
import { useSandbox } from "./helpers.ts";

const sandbox = useSandbox();

async function candidates(...words: string[]): Promise<string[]> {
  const output = await complete(words, sandbox.context());
  return output === "" ? [] : output.split("\n");
}

beforeEach(async () => {
  await sandbox.mkdirs("app/src/main", "app/src/test", "app/scripts", "app/.git");
  await writeFile(join(sandbox.dir, "app", "setup.sh"), "");
  await add("app", sandbox.context(join(sandbox.dir, "app")));
  await add("api", sandbox.context(join(sandbox.dir, "app", "src")));
});

describe("complete", () => {
  it("offers commands and aliases for the first word, filtered by it", async () => {
    assert.deepEqual(await candidates(""), [...COMMAND_NAMES, "api", "app"]);
    assert.deepEqual(await candidates("a"), ["add", "api", "app"]);
  });

  it("offers sub directories after '<alias>/'", async () => {
    assert.deepEqual(await candidates("app/s"), ["app/scripts/", "app/src/"]);
    assert.deepEqual(await candidates("app/src/m"), ["app/src/main/"]);
    assert.deepEqual(await candidates("app/."), ["app/.git/"]);
    assert.deepEqual(await candidates("nope/"), []);
    assert.deepEqual(await candidates("app/nope/"), []);
  });

  it("offers aliases for the first argument of add, del, and ch", async () => {
    for (const command of ["add", "del", "ch"]) {
      assert.deepEqual(await candidates(command, ""), ["api", "app"]);
      assert.deepEqual(await candidates(command, "app", ""), []);
    }
  });

  it("offers aliases and sub directories for path", async () => {
    assert.deepEqual(await candidates("path", "ap"), ["api", "app"]);
    assert.deepEqual(await candidates("path", "api/"), ["api/main/", "api/test/"]);
  });

  it("alternates aliases and cwd directories for set", async () => {
    assert.deepEqual(await candidates("set", ""), ["api", "app"]);
    assert.deepEqual(await candidates("set", "x", "app/s"), ["app/scripts/", "app/src/"]);
    assert.deepEqual(await candidates("set", "x", "app", "a"), ["api", "app"]);
  });

  it("offers order flags for list and nothing for gc", async () => {
    assert.deepEqual(await candidates("list", "-"), LIST_FLAGS);
    assert.deepEqual(await candidates("gc", ""), []);
  });
});
