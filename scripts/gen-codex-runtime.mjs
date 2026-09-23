#!/usr/bin/env node
// Package canonical production modules; never maintain a second policy copy.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const output = path.join(root, 'rdm-core/src/templates/codex-runtime');
const files = [
  ['scripts/rdm-codex.mjs', 'rdm-codex.mjs'],
  ...['codex-runtime', 'codex-runtime-estimate', 'codex-runtime-state', 'codex-process']
    .map(name => [`scripts/lib/${name}.mjs`, `lib/${name}.mjs`]),
  ...['review', 'plan-review', 'estimate']
    .map(name => [`.claude/workflows/lib/${name}.mjs`, `shared/${name}.mjs`]),
];
for (const [source, destination] of files) {
  const content = fs.readFileSync(path.join(root, source), 'utf8')
    .replaceAll('../../.claude/workflows/lib/', '../shared/');
  const target = path.join(output, destination);
  fs.mkdirSync(path.dirname(target), {recursive: true});
  fs.writeFileSync(target, content);
}
