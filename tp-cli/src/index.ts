#!/usr/bin/env node

import { argv, cwd } from "node:process";
import { main } from "./cli.ts";
import { TpError } from "./errors.ts";
import { getConfigFile, getDataFile, loadConfig } from "./store.ts";

if (import.meta.main) {
  try {
    const config = await loadConfig(getConfigFile());
    console.log(await main(argv.slice(2), { cwd: cwd(), dataFile: getDataFile(), config }));
  } catch (error) {
    if (!(error instanceof TpError)) throw error;
    console.error(error.message);
    process.exitCode = 1;
  }
}
