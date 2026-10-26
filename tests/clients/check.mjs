// SPDX-License-Identifier: Apache-2.0
// As check.py, with the JavaScript library @datumesh/verify.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { checkEnvelope, keysFromResponse } from '@datumesh/verify';

const pkg = createRequire(import.meta.url)('@datumesh/verify/package.json');
let total = 0, bad = 0, refused = 0;
for (const line of readFileSync(process.argv[2], 'utf8').split('\n').filter((l) => l.length > 0)) {
  const c = JSON.parse(line);
  const keys = Object.keys(c.keys).sort().map((h) => keysFromResponse(h, c.keys[h].https.status, c.keys[h].https.body));
  const r = checkEnvelope(c.status, c.input, keys, c.request);
  const got = r.ok ? 'ok' : r.code;
  total += 1;
  if (got !== 'ok') refused += 1;
  if (got !== c.expect) {
    bad += 1;
    console.log(`${c.id}: got ${got}, want ${c.expect}: ${r.detail}`);
  }
}
console.log(`javascript @datumesh/verify ${pkg.version}: ${total} cases, ${refused} refused, ${bad} unexpected`);
process.exit(bad ? 1 : 0);
