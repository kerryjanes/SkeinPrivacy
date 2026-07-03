// Codama codegen: turn each Anchor IDL in `target/idl/*.json` into a typed
// @solana/kit client under `sdk/src/generated/<program>`.
//
// Scaffold behaviour: until the first program ships an IDL (M1), there is
// nothing to generate and this exits cleanly so `pnpm build` stays green.

import { existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const idlDir = join(here, '..', 'target', 'idl');
const outDir = join(here, 'src', 'generated');

const idls = existsSync(idlDir) ? readdirSync(idlDir).filter((f) => f === 'weft.json') : [];

if (idls.length === 0) {
  console.log('[codama] No weft IDL in target/idl yet — nothing to generate.');
  process.exit(0);
}

const { createFromRoot } = await import('codama');
const { rootNodeFromAnchor } = await import('@codama/nodes-from-anchor');
const { renderVisitor } = await import('@codama/renderers-js');

mkdirSync(outDir, { recursive: true });

for (const file of idls) {
  const name = file.replace(/\.json$/, '');
  const idl = JSON.parse(readFileSync(join(idlDir, file), 'utf8'));
  const codama = createFromRoot(rootNodeFromAnchor(idl));
  codama.accept(renderVisitor(join(outDir, name)));
  console.log(`[codama] generated client → src/generated/${name}`);
}
