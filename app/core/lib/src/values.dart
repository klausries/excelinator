/// Values as they are shown and typed, and places as they are written.
library;

import 'backend.dart';

/// The members of a JSON list as objects.
List<Json> list(Object? v) => [for (final x in (v as List? ?? const [])) (x as Map).cast<String, dynamic>()];

/// A value of a table as it is shown.
String shown(Object? v) {
  if (v == null) return '';
  if (v is! num) return '$v';
  if (!v.isFinite) return '$v';
  if (v == v.roundToDouble() && v.abs() < 1e15) return v.round().toString();
  return ((v * 1e6).round() / 1e6).toString();
}

/// What was typed, as the value it stands for: nothing, a truth, a number or the text.
Object? typed(String text, [String? dtype]) {
  final t = text.trim();
  if (t.isEmpty) return null;
  final truth = RegExp(r'^(true|false|wahr|falsch)$', caseSensitive: false);
  if (truth.hasMatch(t) && (dtype == null || dtype.isEmpty || dtype == 'Boolean')) return RegExp(r'^(true|wahr)$', caseSensitive: false).hasMatch(t);
  final n = dtype == 'String' ? null : double.tryParse(t.replaceFirst(',', '.'));
  if (n == null || n.isNaN) return text;
  return n == n.roundToDouble() && n.abs() < 1e15 ? n.round() : n;
}

/// The text of a column's filter as the server takes it: `leer`, an operator with a
/// value, or text the column is to contain.
Json filterOf(String column, String text) {
  final t = text.trim();
  if (RegExp(r'^(leer|empty)$', caseSensitive: false).hasMatch(t)) return {'column': column, 'op': 'empty'};
  if (RegExp(r'^(nicht leer|not empty)$', caseSensitive: false).hasMatch(t)) return {'column': column, 'op': 'not empty'};
  final m = RegExp(r'^(<=|>=|!=|=|<|>)\s*(.*)$').firstMatch(t);
  return m != null ? {'column': column, 'op': m[1], 'value': m[2]} : {'column': column, 'op': 'contains', 'value': t};
}

/// The letters of a column: 0 is A.
String colName(int n) {
  var s = '';
  for (n += 1; n > 0; n = (n - 1) ~/ 26) {
    s = String.fromCharCode(65 + (n - 1) % 26) + s;
  }
  return s;
}

/// A range as (first row, first column, last row, last column), counted from 0.
(int, int, int, int)? rangeOf(String ref) {
  final m = RegExp(r'^([A-Z]+)(\d+):([A-Z]+)(\d+)$').firstMatch(ref);
  if (m == null) return null;
  int col(String t) => t.codeUnits.fold(0, (n, ch) => n * 26 + ch - 64) - 1;
  return (int.parse(m[2]!) - 1, col(m[1]!), int.parse(m[4]!) - 1, col(m[3]!));
}

/// The three levels a store is seen on.
enum Level {
  data('Daten'),
  model('Modell'),
  book('Mappe'),
  program('Programm');

  const Level(this.label);
  final String label;
}

/// Where the app is: a store, a level of it, and a table or sheet. As an address:
/// `#/<store>/<level>/<item>`.
class Place {
  const Place([this.store, this.level = Level.data, this.item]);
  final String? store;
  final Level level;
  final String? item;

  factory Place.parse(String hash) {
    final parts = hash.split('/').skip(1).map(Uri.decodeComponent).toList();
    String? at(int i) => parts.length > i && parts[i].isNotEmpty ? parts[i] : null;
    final level = Level.values.where((l) => l.name == at(1)).firstOrNull ?? Level.data;
    return Place(at(0), level, parts.length > 2 ? at(2) == null ? null : parts.skip(2).join('/') : null);
  }

  String get hash => '#/${[store, if (store != null) level.name, if (store != null) item].nonNulls.map(Uri.encodeComponent).join('/')}';

  @override
  bool operator ==(Object other) => other is Place && other.store == store && other.level == level && other.item == item;

  @override
  int get hashCode => Object.hash(store, level, item);

  @override
  String toString() => hash;
}

const kinds = {'table': 'Tabelle', 'cells': 'Einzelzellen', 'block': 'Block', 'stack': 'Stapel'};

String kindOf(Object? kind) => kinds[kind] ?? '$kind';
