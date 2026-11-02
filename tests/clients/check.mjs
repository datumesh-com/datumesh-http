// SPDX-License-Identifier: Apache-2.0
// As check.py, with the JavaScript library @datumesh/verify.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { checkBatch, checkEnvelope, checkSlicePage, keysFromResponse } from '@datumesh/verify';

const pkg = createRequire(import.meta.url)('@datumesh/verify/package.json');
let total = 0, bad = 0, refused = 0;
for (const line of readFileSync(process.argv[2], 'utf8').split('\n').filter((l) => l.length > 0)) {
  const c = JSON.parse(line);
  const keys = Object.keys(c.keys).sort().map((h) => keysFromResponse(h, c.keys[h].https.status, c.keys[h].https.body));
  let got, detail;
  if (c.kind === 'batch') {
    const b = checkBatch(c.status, c.input, keys, c.requests);
    const failing = b.ok ? (b.items ?? []).find((i) => !i.ok) : null;
    got = !b.ok ? b.code : failing ? failing.code : 'ok';
    detail = failing ? failing.detail : b.detail;
  } else {
    const r = c.kind === 'page' ? checkSlicePage(c.status, c.input, keys, c.request) : checkEnvelope(c.status, c.input, keys, c.request);
    got = r.ok ? 'ok' : r.code;
    detail = r.detail;
  }
  total += 1;
  if (got !== 'ok') refused += 1;
  if (got !== c.expect) {
    bad += 1;
    console.log(`${c.id}: got ${got}, want ${c.expect}: ${detail}`);
  }
}
console.log(`javascript @datumesh/verify ${pkg.version}: ${total} cases, ${refused} refused, ${bad} unexpected`);
process.exit(bad ? 1 : 0);
