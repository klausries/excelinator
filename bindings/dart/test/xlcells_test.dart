import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:xlcells/xlcells.dart';

void main() {
  test('pyExcelerator-shaped round trip', () {
    final w = Workbook();
    final ws = w.addSheet('Hey, Dude');
    final st = XFStyle()
      ..numFormatStr = 'D-MMM-YY'
      ..font.bold = true;
    ws.write(0, 0, 'text');
    ws.write(0, 1, 42);
    ws.write(1, 0, true);
    ws.write(1, 1, '');
    ws.write(1, 2, const Formula('SUM(B1;10)'));
    ws.write(2, 0, DateTime(2005, 7, 20, 12), st);
    ws.writeMerge(4, 5, 0, 2, 'Größe 😀');
    final bytes = w.toBytes();

    final sheets = parseXls(bytes);
    expect(sheets.single.$1, 'Hey, Dude');
    expect(sheets.single.$2, {(0, 0): 'text', (0, 1): 42.0, (1, 0): true, (2, 0): 38553.5, (4, 0): 'Größe 😀'});

    final t = read(bytes);
    expect(t.length, 6);
    expect(t.isDate, [0, 0, 0, 0, 1, 0]);
    expect(t.formula[3], 'SUM(B1,10)');
    expect(t.text[0], 'text');
    expect(t.numFormat[t.style[4]], 'D-MMM-YY');
  });

  test('errors are exceptions', () {
    expect(() => Workbook().toBytes(), throwsA(isA<XlError>().having((e) => e.message, 'message', contains('at least one sheet'))));
    expect(() => read(Uint8List.fromList([1, 2, 3])), throwsA(isA<XlError>()));
    expect(() => Writer().write(3, 0, 0, CellKind.number, num: 1), throwsA(isA<XlError>()));
  });

  test('empty workbook reads as empty table', () {
    final w = Workbook()..addSheet('leer');
    final t = read(w.toBytes());
    expect(t.length, 0);
    expect(t.sheetNames, ['leer']);
  });
}
