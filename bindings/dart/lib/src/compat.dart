/// The interface of pyExcelerator for Dart.
library;

import 'dart:typed_data';

import 'core.dart';

/// A formula without the leading `=`; arguments separated by `;` or `,`.
/// [value] is the result to store with it (optional).
class Formula {
  final String text;
  final Object? value;
  const Formula(this.text, [this.value]);
}

/// An error value such as `#N/A`.
class ErrorCode {
  final String text;
  const ErrorCode(this.text);
}

class Font {
  bool bold = false;
  bool italic = false;
}

class XFStyle {
  String numFormatStr = 'General';
  final Font font = Font();
}

(int, double, String?, String?) _cell(Object? label) => switch (label) {
      Formula f => switch (f.value == null ? (CellKind.blank, 0.0, null, null) : _cell(f.value)) {
          (final k, final n, final t, _) => (k, n, t, f.text),
        },
      ErrorCode e => (CellKind.error, 0.0, e.text, null),
      null || '' => (CellKind.blank, 0.0, null, null),
      bool b => (CellKind.boolean, b ? 1.0 : 0.0, null, null),
      String s => (CellKind.text, 0.0, s, null),
      num n => (CellKind.number, n.toDouble(), null, null),
      DateTime d => (
          CellKind.number,
          dateSerial(d.year, d.month, d.day, d.hour, d.minute, d.second, d.millisecond * 1000 + d.microsecond) ??
              (throw ArgumentError.value(d, 'label', 'date cannot be stored in a workbook')),
          null,
          null
        ),
      _ => throw ArgumentError.value(label, 'label', 'cannot be written into a cell'),
    };

class Worksheet {
  final String name;
  final Workbook _wb;
  final int _index;
  Worksheet._(this._wb, this.name, this._index);

  /// `ws.write(r, c, label, style)`: label is a String, num, bool, DateTime,
  /// [Formula], [ErrorCode] or null.
  void write(int r, int c, [Object? label = '', XFStyle? style]) {
    final (kind, num, text, formula) = _cell(label);
    _wb._w.write(_index, r, c, kind, num: num, text: text, formula: formula, style: _wb._style(style));
  }

  void merge(int r1, int r2, int c1, int c2) => _wb._w.merge(_index, r1, r2, c1, c2);

  void writeMerge(int r1, int r2, int c1, int c2, [Object? label = '', XFStyle? style]) {
    merge(r1, r2, c1, c2);
    write(r1, c1, label, style);
  }

  /// Width in 1/256 of a character, as `ws.col(c).width = w` in pyExcelerator.
  void setColWidth(int c, int width) => _wb._w.setColWidth(_index, c, width);
}

class Workbook {
  final Writer _w = Writer();
  final Map<(String, bool, bool), int> _styles = {};
  final List<Worksheet> _sheets = [];

  int _style(XFStyle? style) {
    if (style == null) return 0;
    final fmt = style.numFormatStr.toLowerCase() == 'general' ? 'General' : style.numFormatStr;
    return _styles.putIfAbsent((fmt, style.font.bold, style.font.italic), () => _w.addStyle(numFormat: fmt, bold: style.font.bold, italic: style.font.italic));
  }

  Worksheet addSheet(String name) {
    final ws = Worksheet._(this, name, _w.addSheet(name));
    _sheets.add(ws);
    return ws;
  }

  Worksheet getSheet(int index) => _sheets[index];

  /// The xlsx file (pyExcelerator: `save`).
  Uint8List toBytes() => _w.toBytes();
}

/// Per sheet its name and the values of its non-empty cells by (row, col).
/// Dates arrive as numbers and errors as their text, as in pyExcelerator.
List<(String, Map<(int, int), Object>)> parseXls(Uint8List data) {
  final t = read(data);
  final sheets = [for (final name in t.sheetNames) (name, <(int, int), Object>{})];
  for (var i = 0; i < t.length; i++) {
    final Object? value = switch (t.kind[i]) {
      CellKind.number => t.num[i],
      CellKind.boolean => t.num[i] != 0,
      CellKind.text || CellKind.error => t.text[i],
      _ => null,
    };
    if (value != null) sheets[t.sheet[i]].$2[(t.row[i], t.col[i])] = value;
  }
  return sheets;
}
