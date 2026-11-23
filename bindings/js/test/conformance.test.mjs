// The shared conformance cases: same file bytes and same table as the Rust reference.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Writer, read } from '../node.mjs';

const dir = new URL('../../../tests/conformance/', import.meta.url);

function fnv(data) {
  let h = 0xcbf29ce484222325n;
  for (const b of data) h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return h.toString(16).padStart(16, '0');
}
const text = (f) => (f === '-' ? undefined : f === '=' ? '' : Buffer.from(f, 'hex').toString('utf8'));
const bits = (x) => {
  const b = Buffer.alloc(8);
  b.writeDoubleBE(x);
  return b.toString('hex');
};

function run(lines) {
  const w = new Writer();
  const styles = [0];
  let sheet = 0;
  for (const line of lines) {
    const f = line.split(' ');
    const n = (i) => Number(f[i]);
    if (f[0] === 'date1904') w.setDate1904(f[1] === '1');
    else if (f[0] === 'style') styles.push(w.addStyle(text(f[3]), f[1] === '1', f[2] === '1'));
    else if (f[0] === 'sheet') sheet = w.addSheet(text(f[1]));
    else if (f[0] === 'merge') w.merge(sheet, n(1), n(2), n(3), n(4));
    else if (f[0] === 'width') w.setColWidth(sheet, n(1), n(2));
    else if (f[0] === 'cell') w.write(sheet, n(1), n(2), n(3), Buffer.from(f[4], 'hex').readDoubleBE(), text(f[6]), text(f[7]), styles[n(5)]);
  }
  const data = w.toBytes();
  w.free();
  const t = read(data);
  let dump = t.sheetNames.map((s) => `S\t${s}\n`).join('') + `D\t${t.date1904 ? 1 : 0}\n` + t.numFormat.map((s) => `N\t${s}\n`).join('');
  const { sheet: sh, row, col, kind, num, isDate, style, text: txt, formula } = t;
  for (let i = 0; i < t.length; i++) {
    dump += `C\t${sh[i]}\t${row[i]}\t${col[i]}\t${kind[i]}\t${bits(num[i])}\t${isDate[i]}\t${style[i]}\t${kind[i] === 2 || kind[i] === 4 ? txt[i] : '\\N'}\t${formula[i] ?? '\\N'}\n`;
  }
  t.free();
  return `${data.length}\t${fnv(data)}\t${fnv(Buffer.from(dump, 'utf8'))}`;
}

test('conformance cases', () => {
  const got = [];
  for (const block of readFileSync(new URL('cases.txt', dir), 'utf8').split('\nend\n')) {
    const lines = block.split('\n');
    if (!lines[0].startsWith('case ')) continue;
    got.push(`${lines[0].slice(5)}\t${run(lines.slice(1))}`);
  }
  const expected = readFileSync(new URL('expected.txt', dir), 'utf8').trimEnd().split('\n');
  assert.ok(got.length >= 45);
  assert.deepEqual(got, expected);
});
