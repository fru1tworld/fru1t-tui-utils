import assert from "node:assert/strict";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach } from "node:test";
import type { Context } from "../commands.ts";
import type { TpConfig } from "../store.ts";

export interface Sandbox {
  readonly dir: string;
  readonly dataFile: string;
  // A command context running in cwd (defaults to the sandbox directory)
  readonly context: (cwd?: string, config?: TpConfig) => Context;
  // Creates directories under the sandbox and returns the absolute path of the last one
  readonly mkdirs: (...paths: string[]) => Promise<string>;
}

// Gives every test in the enclosing suite a fresh temporary directory
export function useSandbox(): Sandbox {
  const sandbox = {
    dir: "",
    get dataFile() {
      return join(this.dir, "bookmarks.json");
    },
    context(cwd?: string, config: TpConfig = {}): Context {
      return { cwd: cwd ?? this.dir, dataFile: this.dataFile, config };
    },
    async mkdirs(...paths: string[]): Promise<string> {
      let last = this.dir;
      for (const path of paths) {
        last = join(this.dir, path);
        await mkdir(last, { recursive: true });
      }
      return last;
    },
  };
  beforeEach(async () => {
    sandbox.dir = await realpath(await mkdtemp(join(tmpdir(), "tp-test-")));
  });
  afterEach(() => rm(sandbox.dir, { recursive: true, force: true }));
  return sandbox;
}

export function assertIncludes(actual: string, expected: string): void {
  assert.ok(
    actual.includes(expected),
    `expected ${JSON.stringify(actual)} to include ${JSON.stringify(expected)}`,
  );
}

export async function assertRejects(promise: Promise<unknown>, message: string): Promise<void> {
  await assert.rejects(promise, (error: unknown) => {
    assert.ok(error instanceof Error);
    assertIncludes(error.message, message);
    return true;
  });
}
