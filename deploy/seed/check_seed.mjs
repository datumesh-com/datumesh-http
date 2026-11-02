// SPDX-License-Identifier: Apache-2.0
// The seed checks' JavaScript part: one verse of each corpus (and edition) fetched over HTTPS,
// its envelope checked and its receipt verified at `signed` with the blob content (SPEC 10.1
// step 7.7) by @datumesh/verify 0.4.2, the blob from /blob/<sha256>.
//   node check_seed.mjs <base URL> <verses.json written by check_seed.py> <API key file>
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { checkEnvelope, keysFromResponse, verify } from '@datumesh/verify';

const pkg = createRequire(import.meta.url)('@datumesh/verify/package.json');
const [base, file, keyFile] = process.argv.slice(2);
const host = new URL(base).host;
const auth = { Authorization: `Bearer ${readFileSync(keyFile, 'utf8').trim()}` };
const kd = await fetch(`${base}/.well-known/datumesh.json`, { redirect: 'manual' });
const keys = keysFromResponse(host, kd.status, await kd.text());
const results = [];
for (const v of JSON.parse(readFileSync(file, 'utf8'))) {
  const res = await fetch(`${base}${v.path}`, { redirect: 'manual', headers: auth });
  const body = await res.text();
  const r = checkEnvelope(res.status, body, [keys], { cell_id: v.cell_id, mesh: host });
  const b = await fetch(`${base}/blob/${v.blob}`, { redirect: 'manual', headers: auth });
  const blob = new Uint8Array(await b.arrayBuffer());
  const rec = JSON.parse(body).receipt;
  const ver = verify(rec, [keys], null, blob);
  const ok = keys.ok && r.ok && r.outcome === 'ok' && r.trust === 'primary' && ver.ok && ver.level === 'signed' && ver.blobChecked;
  results.push({ corpus: v.corpus, envelope: r.ok ? `${r.outcome} ${r.trust}` : r.code, receipt: ver.ok ? ver.level : ver.code, blob_checked: ver.blobChecked, bytes: blob.length, ok });
}
const ok = results.length > 0 && results.every((x) => x.ok);
console.log(JSON.stringify({ library: `@datumesh/verify ${pkg.version}`, key_document: keys.ok, results, ok }));
process.exit(ok ? 0 : 1);
