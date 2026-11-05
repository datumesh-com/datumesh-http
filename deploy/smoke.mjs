// SPDX-License-Identifier: Apache-2.0
// The smoke suite's JavaScript part: a computed cell's envelope and the key document fetched
// over HTTPS, checked and verified with @datumesh/verify 0.4.3 (trust computed against the host
// asked), and a scripture verse verified with its blob content (10.1 step 7.7).
//   node smoke.mjs <base URL> <receipt.json written by smoke.py> <verse.json written by smoke.py>   (NODE_EXTRA_CA_CERTS for a private CA)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { checkEnvelope, keysFromResponse, verify, cellHash } from '@datumesh/verify';

const pkg = createRequire(import.meta.url)('@datumesh/verify/package.json');
const [base, file, verseFile] = process.argv.slice(2);
const host = new URL(base).host;
const rec = JSON.parse(readFileSync(file, 'utf8'));
const kd = await fetch(`${base}/.well-known/datumesh.json`, { redirect: 'manual' });
const keys = keysFromResponse(host, kd.status, await kd.text());
const c = rec.cell;
const args = Object.keys(c.args).sort().map((k) => `${k}=${c.args[k].value}`).join('&');
const res = await fetch(`${base}/cell/${c.ns}/${c.fn}?${args}`, { redirect: 'manual' });
const body = await res.text();
const r = checkEnvelope(res.status, body, [keys], { cell_id: cellHash(c), mesh: host });
const v = verify(JSON.parse(body).receipt, [keys]);
const verse = JSON.parse(readFileSync(verseFile, 'utf8'));
const vb = await fetch(`${base}/blob/${verse.receipt.value.slice(0, 64)}`, { redirect: 'manual' });
const blob = new Uint8Array(await vb.arrayBuffer());
const vv = verify(verse.receipt, [keys], null, blob);
const ok = keys.ok && r.ok && r.outcome === 'ok' && r.trust === 'computed' && v.ok && v.level === 'signed' && vv.ok && vv.blobChecked;
console.log(JSON.stringify({ library: `@datumesh/verify ${pkg.version}`, key_document: keys.ok, envelope: r.ok ? `${r.outcome} ${r.trust}` : r.code, receipt: v.ok ? v.level : v.code, verse: vv.ok ? `${vv.level}, blob checked ${vv.blobChecked}` : vv.code, ok }));
process.exit(ok ? 0 : 1);
