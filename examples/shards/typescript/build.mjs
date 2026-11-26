// Turns main.ts into the single executable file brok installs. Node removes
// the types itself (Node 22.13 or newer), so there is nothing to download.
// A shard with npm dependencies would bundle them here, for example with esbuild.
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';

mkdirSync('dist', { recursive: true });
const code = stripTypeScriptTypes(readFileSync('main.ts', 'utf8'));
writeFileSync('dist/todo-typescript', `#!/usr/bin/env node\n${code}`);
chmodSync('dist/todo-typescript', 0o755);
