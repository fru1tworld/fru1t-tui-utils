import { readFile } from "node:fs/promises";
import { CommandError } from "./errors.ts";
import { type Shell, SUPPORTED_SHELLS, USAGE } from "./usage.ts";

const isShell = (value: string | undefined): value is Shell =>
  SUPPORTED_SHELLS.some((shell) => shell === value);

export const shellScriptFile = (shell: Shell): URL => new URL(`../tp.${shell}`, import.meta.url);

export async function shellInit(shell: string | undefined): Promise<string> {
  if (!isShell(shell)) throw new CommandError(USAGE.init);
  return (await readFile(shellScriptFile(shell), "utf-8")).trimEnd();
}
