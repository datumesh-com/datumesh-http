// SPDX-License-Identifier: Apache-2.0
// The production smoke (step 1.7a item 7) with @datumesh/verify 0.5.1:
//   node smoke_prod.mjs [<base URL> [<namespace>]]   (defaults https://datumesh.com, datumesh.com/scripture)
// The first and last verse of each corpus in each edition, verified at `signed` with the blob
// checked; the verse and chapter counts from the mesh's count cells against the source files' own.
import * as dm from '@datumesh/verify';

const BASE = (process.argv[2] ?? 'https://datumesh.com').replace(/\/$/, '');
const NS = process.argv[3] ?? 'datumesh.com/scripture';
const HOST = NS.split('/')[0];
const failed = [];
const get = async (p) => { const r = await fetch(BASE + p); return [r.status, new Uint8Array(await r.arrayBuffer())]; };
const check = (name, ok, detail = {}) => { console.log(JSON.stringify({ check: name, ok: !!ok, ...detail })); if (!ok) failed.push(name); };
const keys = dm.keysFromResponse(HOST, ...(await get('/.well-known/datumesh.json')));
check('key document', keys.ok);
const cell = async (call) => {
  const [s, b] = await get('/cell/' + encodeURIComponent(call).replaceAll('%2F', '/'));
  return [s, JSON.parse(new TextDecoder().decode(b))];
};
for (const [corpus, book, ch, v, ed] of [['tanakh', 'Gen', 1, 1, 'uxlc'], ['tanakh', '2Chr', 36, 23, 'uxlc'], ['tanakh', 'Neh', 13, 31, 'uxlc'],
  ['nt', 'Matt', 1, 1, 'sblgnt'], ['nt', 'Rev', 22, 21, 'sblgnt'], ['quran', 'Quran', 1, 1, 'uthmani'],
  ['quran', 'Quran', 114, 6, 'uthmani'], ['quran', 'Quran', 1, 1, 'simple'], ['quran', 'Quran', 114, 6, 'simple']]) {
  const [s, e] = await cell(`${NS}/verse(${corpus}, "${book}", ${ch}, ${v}).text(${ed})`) // a book code starting with a digit is quoted;
  const rec = e.receipt;
  const content = rec ? (await get('/blob/' + rec.value.split(':')[0]))[1] : null;
  const r = rec ? dm.verify(rec, [keys], null, content) : null;
  check(`${corpus} ${book} ${ch}:${v} ${ed}`, s === 200 && e.trust === 'primary' && r?.ok && r.level === 'signed' && r.blobChecked,
    { bytes: content?.length, level: r?.level, blobChecked: r?.blobChecked });
}
// The members of a slice, every page, each receipt verified (paths of `next` read from this base).
const members = async (call) => {
  let path = '/slice/' + encodeURIComponent(call).replaceAll('%2F', '/'), out = [];
  while (path) {
    const [s, b] = await get(path);
    if (s !== 200) return [];
    const page = JSON.parse(new TextDecoder().decode(b));
    for (const m of page.members ?? []) { if (!dm.verify(m.receipt, [keys]).ok) return []; out.push(m.receipt); }
    path = page.next ? (() => { const u = new URL(page.next); return u.pathname + u.search; })() : null;
  }
  return out;
};
for (const [corpus, nv, nc, nb] of [['tanakh', 23213, 929, 39], ['nt', 7939, 260, 27], ['quran', 6236, 114, 1]]) {
  const cc = await members(`${NS}/chapter_count(corpus: "${corpus}", book: *)`);
  const vc = await members(`${NS}/verse_count(corpus: "${corpus}", book: *, chapter: *)`);
  const chapters = cc.reduce((a, r) => a + Number(r.value), 0), verses = vc.reduce((a, r) => a + Number(r.value), 0);
  check(`${corpus}: verse and chapter counts equal the source files' own`, verses === nv && chapters === nc && cc.length === nb && vc.length === nc,
    { verses, chapters, books: cc.length });
}
console.log(JSON.stringify({ javascript: `@datumesh/verify ${dm.VERSION}`, failed }));
process.exit(failed.length ? 1 : 0);
