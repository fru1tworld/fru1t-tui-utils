import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../..", import.meta.url));

describe("Shell integration files", () => {
  describe("tp.bash (Bash)", () => {
    const filePath = path.join(root, "tp.bash");

    it("exists", () => {
      expect(fs.existsSync(filePath)).toBe(true);
    });

    it("contains tp wrapper function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("tp()");
      expect(content).toContain("tp-cli");
      expect(content).toContain("__TP_CD__:");
      expect(content).toContain('cd -- "');
    });

    it("contains Bash completion function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("_tp_completions()");
      expect(content).toContain("COMP_WORDS");
      expect(content).toContain("COMPREPLY");
      expect(content).toContain("--completions");
    });

    it("registers completion", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("complete -F _tp_completions tp");
    });

    it("completes list order flags", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("-u --utf8 -r --recent");
    });
  });

  describe("tp.zsh (Zsh)", () => {
    const filePath = path.join(root, "tp.zsh");

    it("exists", () => {
      expect(fs.existsSync(filePath)).toBe(true);
    });

    it("contains tp wrapper function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("tp()");
      expect(content).toContain("tp-cli");
      expect(content).toContain("__TP_CD__:");
      expect(content).toContain('cd -- "');
    });

    it("contains Zsh completion function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("_tp_completions_zsh()");
      expect(content).toContain("_values");
      expect(content).toContain("--completions");
    });

    it("registers completion with compdef", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("compdef _tp_completions_zsh tp");
    });

    it("completes list order flags", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("_values 'order' -u --utf8 -r --recent");
    });
  });

  describe("tp.nu (Nushell)", () => {
    const filePath = path.join(root, "tp.nu");

    it("exists", () => {
      expect(fs.existsSync(filePath)).toBe(true);
    });

    it("contains tp wrapper function with --env", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("def --env tp");
      expect(content).toContain("tp-cli");
      expect(content).toContain("__TP_CD__:");
      expect(content).toContain("str starts-with");
      expect(content).toContain("str substring");
      expect(content).toContain("cd");
    });

    it("contains completion function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("nu-complete tp commands");
      expect(content).toContain("--completions");
    });

    it("wires the completer to the tp arguments", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain('...args: string@"nu-complete tp commands"');
    });

    it("defines the completer before tp uses it", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content.indexOf('def "nu-complete tp commands"')).toBeLessThan(
        content.indexOf("def --env tp"),
      );
    });

    it("includes all tp subcommands", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      const commands = ["add", "set", "del", "ch", "gc", "list", "help"];
      for (const cmd of commands) {
        expect(content).toContain(`"${cmd}"`);
      }
    });
  });

  describe("tp.fish (Fish)", () => {
    const filePath = path.join(root, "tp.fish");

    it("exists", () => {
      expect(fs.existsSync(filePath)).toBe(true);
    });

    it("contains tp wrapper function", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("function tp");
      expect(content).toContain("tp-cli");
      expect(content).toContain("__TP_CD__:");
      expect(content).toContain("string match");
      expect(content).toContain("string replace");
      expect(content).toContain("cd");
    });

    it("contains completion setup", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("complete -c tp");
      expect(content).toContain("__fish_use_subcommand");
      expect(content).toContain("--completions");
    });

    it("registers all tp subcommands", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      const commands = ["add", "set", "del", "ch", "gc", "list", "help"];
      for (const cmd of commands) {
        expect(content).toContain(cmd);
      }
    });

    it("provides alias completion for add, del, and ch", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain("__fish_seen_subcommand_from add del ch");
    });

    it("completes list order flags", () => {
      const content = fs.readFileSync(filePath, "utf-8");
      expect(content).toContain(
        "__fish_seen_subcommand_from list' -s u -l utf8",
      );
      expect(content).toContain(
        "__fish_seen_subcommand_from list' -s r -l recent",
      );
    });
  });

  describe("wrapper exit status", () => {
    const cases = [
      {
        shell: "bash",
        wrapper: "tp.bash",
        script:
          'tp-cli() { printf failure; return 7; }; source "$1"; tp missing >/dev/null 2>&1',
      },
      {
        shell: "zsh",
        wrapper: "tp.zsh",
        script:
          'function tp-cli { print -n failure; return 7 }; source "$1"; tp missing >/dev/null 2>&1',
      },
      {
        shell: "fish",
        wrapper: "tp.fish",
        script:
          "function tp-cli; printf failure; return 7; end; source $argv[2]; tp missing >/dev/null 2>&1",
      },
    ] as const;

    for (const testCase of cases) {
      it(`${testCase.shell} preserves tp-cli failures`, () => {
        const available = spawnSync(testCase.shell, ["--version"]);
        if (available.error) return;

        const result = spawnSync(testCase.shell, [
          "-c",
          testCase.script,
          "tp-wrapper-test",
          path.join(root, testCase.wrapper),
        ]);
        expect(result.status).toBe(7);
      });
    }
  });
});
