// The shared conformance cases: same file bytes and same table as the Rust reference.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:xlcells/xlcells.dart';

String fnv(List<int> data) {
  var h = BigInt.parse('cbf29ce484222325', radix: 16);
  final prime = BigInt.parse('100000001b3', radix: 16);
  final mask = (BigInt.one << 64) - BigInt.one;
  for (final b in data) {
    h = ((h ^ BigInt.from(b)) * prime) & mask;
  }
  return h.toRadixString(16).padLeft(16, '0');
}

Uint8List unhex(String s) => Uint8List.fromList([for (var i = 0; i < s.length; i += 2) int.parse(s.substring(i, i + 2), radix: 16)]);
String? text(String f) => f == '-' ? null : (f == '=' ? '' : utf8.decode(unhex(f)));
String bits(double x) => (ByteData(8)..setFloat64(0, x)).buffer.asUint8List().map((b) => b.toRadixString(16).padLeft(2, '0')).join();

String run(List<String> lines) {
  final w = Writer();
  final styles = [0];
  var sheet = 0;
  for (final line in lines) {
    final f = line.split(' ');
    int n(int i) => int.parse(f[i]);
    switch (f[0]) {
      case 'date1904':
        w.setDate1904(f[1] == '1');
      case 'style':
        styles.add(w.addStyle(numFormat: text(f[3])!, bold: f[1] == '1', italic: f[2] == '1'));
      case 'sheet':
        sheet = w.addSheet(text(f[1])!);
      case 'merge':
        w.merge(sheet, n(1), n(2), n(3), n(4));
      case 'width':
        w.setColWidth(sheet, n(1), n(2));
      case 'cell':
        w.write(sheet, n(1), n(2), n(3), num: ByteData.sublistView(unhex(f[4])).getFloat64(0), text: text(f[6]), formula: text(f[7]), style: styles[n(5)]);
    }
  }
  final data = w.toBytes();
  final t = read(data);
  final dump = StringBuffer();
  for (final s in t.sheetNames) {
    dump.write('S\t$s\n');
  }
  dump.write('D\t${t.date1904 ? 1 : 0}\n');
  for (final s in t.numFormat) {
    dump.write('N\t$s\n');
  }
  for (var i = 0; i < t.length; i++) {
    final txt = t.kind[i] == 2 || t.kind[i] == 4 ? t.text[i] : r'\N';
    dump.write('C\t${t.sheet[i]}\t${t.row[i]}\t${t.col[i]}\t${t.kind[i]}\t${bits(t.num[i])}\t${t.isDate[i]}\t${t.style[i]}\t$txt\t${t.formula[i] ?? r'\N'}\n');
  }
  return '${data.length}\t${fnv(data)}\t${fnv(utf8.encode(dump.toString()))}';
}

void main() {
  test('conformance cases', () {
    final dir = Directory.current.uri.resolve('../../tests/conformance/');
    final got = <String>[];
    for (final block in File.fromUri(dir.resolve('cases.txt')).readAsStringSync().split('\nend\n')) {
      final lines = block.split('\n');
      if (!lines[0].startsWith('case ')) continue;
      got.add('${lines[0].substring(5)}\t${run(lines.sublist(1))}');
    }
    final expected = File.fromUri(dir.resolve('expected.txt')).readAsStringSync().trimRight().split('\n');
    expect(got.length, greaterThanOrEqualTo(45));
    expect(got, expected);
  });
}
