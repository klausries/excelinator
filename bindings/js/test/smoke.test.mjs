import test from 'node:test';
import assert from 'node:assert/strict';
import { Workbook, Formula, XFStyle, parse_xls, read } from '../node.mjs';

test('pyExcelerator-shaped round trip', () => {
  const w = new Workbook();
  const ws = w.add_sheet('Hey, Dude');
  const st = new XFStyle();
  st.num_format_str = 'D-MMM-YY';
  st.font.bold = true;
  ws.write(0, 0, 'text');
  ws.write(0, 1, 42);
  ws.write(1, 0, true);
  ws.write(1, 1, '');
  ws.write(1, 2, new Formula('SUM(B1;10)'));
  ws.write(2, 0, new Date(Date.UTC(2005, 6, 20, 12)), st);
  ws.write_merge(4, 5, 0, 2, 'Größe 😀');
  const bytes = w.to_bytes();
  const sheets = parse_xls(bytes);
  assert.equal(sheets[0][0], 'Hey, Dude');
  assert.deepEqual([...sheets[0][1]], [['0,0', 'text'], ['0,1', 42], ['1,0', true], ['2,0', 38553.5], ['4,0', 'Größe 😀']]);
  const t = read(bytes);
  assert.equal(t.length, 6);
  assert.deepEqual([...t.isDate], [0, 0, 0, 0, 1, 0]);
  assert.equal(t.formula[3], 'SUM(B1,10)');
  assert.equal(t.numFormat[t.style[4]], 'D-MMM-YY');
  t.free();
});

test('errors are exceptions', () => {
  assert.throws(() => new Workbook().to_bytes(), /at least one sheet/);
  assert.throws(() => read(new Uint8Array([1, 2, 3])));
});
