#!/usr/bin/env node
import fs from 'node:fs';
import {runRuntime} from './lib/codex-runtime.mjs';
try {
  if (process.argv.length !== 3) throw new Error('Usage: node scripts/rdm-codex.mjs /absolute/run-spec.json');
  const spec=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
  const result=await runRuntime(spec);
  process.stdout.write(JSON.stringify(result,null,2)+'\n');
} catch(error) {process.stderr.write(`Codex runtime failed: ${error.message}\n`);process.exitCode=1;}
