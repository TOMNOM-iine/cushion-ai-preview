import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
const expected = ["apps/desktop/src/wasm/cushion_core.js","apps/desktop/src/wasm/cushion_core.d.ts","apps/desktop/src/wasm/cushion_core_bg.wasm","apps/desktop/src/wasm/cushion_core_bg.wasm.d.ts"];
try {
  const root = new URL('../', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('PUBLIC_PREVIEW_MANIFEST.json', root), 'utf8'));
  if (!manifest.generatedWasmIncluded) throw new Error();
  for (const name of expected) {
    const entry = manifest.files.filter(file => file.path === name);
    if (entry.length !== 1) throw new Error();
    const bytes = await readFile(new URL(name, root));
    if (bytes.length !== entry[0].bytes || createHash('sha256').update(bytes).digest('hex') !== entry[0].sha256) throw new Error();
  }
  console.log('WASM: 4 files match the reviewed manifest.');
} catch { console.error('WASM verification failed; the bundle is missing, changed, or not finalized.'); process.exitCode = 1; }
