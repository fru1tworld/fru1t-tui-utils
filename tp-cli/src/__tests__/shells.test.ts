import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { execa } from "execa";
import { CD_DIRECTIVE_PREFIX } from "../commands.ts";
import { shellInit, shellScriptFile } from "../shell.ts";
import { type Shell, SUPPORTED_SHELLS } from "../usage.ts";
import { assertRejects } from "./helpers.ts";

const wrapperPath = (shell: Shell) => fileURLToPath(shellScriptFile(shell));

async function hasShell(shell: string): Promise<boolean> {
  const { failed } = await execa(shell, ["-c", "exit 0"], { reject: false });
  return !failed;
}

describe("shellInit", () => {
  for (const shell of SUPPORTED_SHELLS) {
    it(`prints the ${shell} wrapper, which delegates cd and completion to tp-cli`, async () => {
      const script = await shellInit(shell);
      assert.equal(script, (await readFile(shellScriptFile(shell), "utf-8")).trimEnd());
      assert.ok(script.includes(CD_DIRECTIVE_PREFIX));
      assert.ok(script.includes("tp-cli --complete"));
    });
  }

  it("rejects missing and unsupported shells", async () => {
    await assertRejects(shellInit(undefined), "Usage: tp-cli init");
    await assertRejects(shellInit("powershell"), "bash|zsh|fish|nu");
  });
});

describe("shell wrappers", () => {
  const failingCli = {
    bash: 'tp-cli() { printf failure; return 7; }; source "$1"; tp missing',
    zsh: 'function tp-cli { print -n failure; return 7 }; source "$1"; tp missing',
    fish: "function tp-cli; printf failure; return 7; end; source $argv[1]; tp missing",
  } as const satisfies Partial<Record<Shell, string>>;

  for (const shell of ["bash", "zsh", "fish"] as const) {
    const script = failingCli[shell];
    it(`${shell} preserves tp-cli failures without printing stdout`, async (t) => {
      if (!(await hasShell(shell))) return t.skip(`${shell} is not installed`);
      const args = shell === "fish" ? [wrapperPath(shell)] : ["tp-wrapper-test", wrapperPath(shell)];
      const { exitCode, stdout } = await execa(shell, ["-c", script, ...args], { reject: false });
      assert.equal(exitCode, 7);
      assert.equal(stdout, "");
    });
  }

  it("bash adds a space only after finished words", async () => {
    const script = [
      "tp-cli() { printf 'app/src/\\nadd\\n'; }",
      'source "$1"',
      "COMP_WORDS=(tp a); COMP_CWORD=1; _tp_completions",
      // biome-ignore lint/suspicious/noTemplateCurlyInString: bash expansion, not a JS template
      'printf "[%s]" "${COMPREPLY[@]}"',
    ].join("\n");
    const { stdout } = await execa("bash", ["-c", script, "tp-wrapper-test", wrapperPath("bash")]);
    assert.equal(stdout, "[app/src/][add ]");
  });
});
