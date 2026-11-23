import 'dart:convert';
import 'dart:ffi';
import 'dart:io' show Platform;
import 'dart:typed_data';

final class _Writer extends Opaque {}

final class _Bytes extends Opaque {}

final class _Table extends Opaque {}

/// A failure reported by the library (invalid workbook, invalid file, ...).
class XlError implements Exception {
  final String message;
  XlError(this.message);
  @override
  String toString() => 'XlError: $message';
}

/// The values of the `kind` column.
abstract final class CellKind {
  static const blank = 0, number = 1, text = 2, boolean = 3, error = 4;
}

DynamicLibrary? _lib;

/// Load the native library from [path]. Without a call the platform's default name
/// (`libxlcells.so`, `libxlcells.dylib`, `xlcells.dll`) is looked up, or the path in
/// the environment variable `XLCELLS_LIB`.
void loadLibrary(String path) => _lib = DynamicLibrary.open(path);

DynamicLibrary get _l => _lib ??= DynamicLibrary.open(Platform.environment['XLCELLS_LIB'] ??
    (Platform.isWindows ? 'xlcells.dll' : Platform.isMacOS ? 'libxlcells.dylib' : 'libxlcells.so'));

typedef _P = Pointer<Uint8>;

late final _alloc = _l.lookupFunction<_P Function(Size), _P Function(int)>('xl_alloc');
late final _dealloc = _l.lookupFunction<Void Function(_P, Size), void Function(_P, int)>('xl_dealloc');
late final _writerNew = _l.lookupFunction<Pointer<_Writer> Function(), Pointer<_Writer> Function()>('xl_writer_new');
late final _writerFreePtr = _l.lookup<NativeFunction<Void Function(Pointer<_Writer>)>>('xl_writer_free');
late final _addSheet = _l.lookupFunction<Int32 Function(Pointer<_Writer>, _P, Size), int Function(Pointer<_Writer>, _P, int)>('xl_writer_add_sheet');
late final _addStyle = _l.lookupFunction<Uint32 Function(Pointer<_Writer>, _P, Size, Bool, Bool), int Function(Pointer<_Writer>, _P, int, bool, bool)>('xl_writer_add_style');
late final _write = _l.lookupFunction<
    Int32 Function(Pointer<_Writer>, Uint32, Uint32, Uint32, Uint8, Double, _P, Size, _P, Size, Uint32),
    int Function(Pointer<_Writer>, int, int, int, int, double, _P, int, _P, int, int)>('xl_writer_write');
late final _merge = _l.lookupFunction<Void Function(Pointer<_Writer>, Uint32, Uint32, Uint32, Uint32, Uint32),
    void Function(Pointer<_Writer>, int, int, int, int, int)>('xl_writer_merge');
late final _setColWidth = _l.lookupFunction<Void Function(Pointer<_Writer>, Uint32, Uint32, Uint32), void Function(Pointer<_Writer>, int, int, int)>('xl_writer_set_col_width');
late final _setDate1904 = _l.lookupFunction<Void Function(Pointer<_Writer>, Bool), void Function(Pointer<_Writer>, bool)>('xl_writer_set_date1904');
late final _toBytes = _l.lookupFunction<Pointer<_Bytes> Function(Pointer<_Writer>, Pointer<Pointer<_Bytes>>),
    Pointer<_Bytes> Function(Pointer<_Writer>, Pointer<Pointer<_Bytes>>)>('xl_writer_to_bytes');
late final _bytesData = _l.lookupFunction<_P Function(Pointer<_Bytes>), _P Function(Pointer<_Bytes>)>('xl_bytes_data');
late final _bytesLen = _l.lookupFunction<Size Function(Pointer<_Bytes>), int Function(Pointer<_Bytes>)>('xl_bytes_len');
late final _bytesFree = _l.lookupFunction<Void Function(Pointer<_Bytes>), void Function(Pointer<_Bytes>)>('xl_bytes_free');
late final _read = _l.lookupFunction<Pointer<_Table> Function(_P, Size, Pointer<Pointer<_Bytes>>),
    Pointer<_Table> Function(_P, int, Pointer<Pointer<_Bytes>>)>('xl_read');
late final _tableFree = _l.lookupFunction<Void Function(Pointer<_Table>), void Function(Pointer<_Table>)>('xl_table_free');
late final _dateSerial = _l.lookupFunction<Double Function(Int32, Uint32, Uint32, Uint32, Uint32, Uint32, Uint32),
    double Function(int, int, int, int, int, int, int)>('xl_date_serial');

int Function(Pointer<_Table>) _size(String name) => _l.lookupFunction<Size Function(Pointer<_Table>), int Function(Pointer<_Table>)>(name);
Pointer<T> Function(Pointer<_Table>) _column<T extends NativeType>(String name) {
  final f = _l.lookupFunction<Pointer<Void> Function(Pointer<_Table>), Pointer<Void> Function(Pointer<_Table>)>(name);
  return (t) => f(t).cast<T>();
}
_P Function(Pointer<_Table>, int, Pointer<Size>) _indexed(String name) =>
    _l.lookupFunction<_P Function(Pointer<_Table>, Size, Pointer<Size>), _P Function(Pointer<_Table>, int, Pointer<Size>)>(name);

/// Runs [body] with [strings] copied into native memory as (pointer, length) pairs;
/// `null` becomes the null pointer.
T _withStrings<T>(List<String?> strings, T Function(List<(_P, int)>) body) {
  final args = <(_P, int)>[];
  try {
    for (final s in strings) {
      if (s == null) {
        args.add((nullptr, 0));
      } else {
        final bytes = utf8.encode(s);
        final p = _alloc(bytes.length);
        p.asTypedList(bytes.length).setAll(0, bytes);
        args.add((p, bytes.length));
      }
    }
    return body(args);
  } finally {
    for (final (p, n) in args) {
      if (p != nullptr) _dealloc(p, n);
    }
  }
}

/// Takes ownership of a native byte object and returns a copy of its content.
Uint8List _take(Pointer<_Bytes> b) {
  try {
    return Uint8List.fromList(_bytesData(b).asTypedList(_bytesLen(b)));
  } finally {
    _bytesFree(b);
  }
}

/// Calls [body] with a slot for an error message; throws [XlError] if it returns null.
Pointer<T> _checked<T extends NativeType>(Pointer<T> Function(Pointer<Pointer<_Bytes>>) body) {
  final slot = _alloc(sizeOf<Pointer<_Bytes>>()).cast<Pointer<_Bytes>>();
  slot.value = nullptr;
  try {
    final result = body(slot);
    if (result == nullptr) {
      throw XlError(slot.value == nullptr ? 'unknown error' : utf8.decode(_take(slot.value)));
    }
    return result;
  } finally {
    _dealloc(slot.cast(), sizeOf<Pointer<_Bytes>>());
  }
}

/// Builds a workbook cell by cell. Rows and columns are 0-based.
final class Writer implements Finalizable {
  static final _finalizer = NativeFinalizer(_writerFreePtr.cast());
  final Pointer<_Writer> _w = _writerNew();

  Writer() {
    _finalizer.attach(this, _w.cast(), detach: this);
  }

  /// Adds a worksheet and returns its index.
  int addSheet(String name) => _withStrings([name], (a) => _addSheet(_w, a[0].$1, a[0].$2));

  /// Registers a style and returns its index (0 is the default style).
  int addStyle({String numFormat = 'General', bool bold = false, bool italic = false}) =>
      _withStrings([numFormat], (a) => _addStyle(_w, a[0].$1, a[0].$2, bold, italic));

  /// Writes one cell; [kind] is a [CellKind], [formula] has no leading `=`.
  void write(int sheet, int row, int col, int kind, {double num = 0, String? text, String? formula, int style = 0}) {
    final rc = _withStrings([text, formula], (a) => _write(_w, sheet, row, col, kind, num, a[0].$1, a[0].$2, a[1].$1, a[1].$2, style));
    if (rc != 0) throw XlError('cannot write cell ($row, $col) of sheet $sheet with kind $kind');
  }

  void merge(int sheet, int r1, int r2, int c1, int c2) => _merge(_w, sheet, r1, r2, c1, c2);

  /// Column width in 1/256 of a character.
  void setColWidth(int sheet, int col, int width) => _setColWidth(_w, sheet, col, width);

  void setDate1904(bool on) => _setDate1904(_w, on);

  /// The xlsx file.
  Uint8List toBytes() => _take(_checked((error) => _toBytes(_w, error)));
}

/// The cells of a workbook as equally long columns (one entry per cell).
final class Table {
  final List<String> sheetNames;
  final bool date1904;
  final Uint32List sheet, row, col, style;

  /// [CellKind] per cell.
  final Uint8List kind;
  final Float64List num;

  /// 1 where the number is a date serial.
  final Uint8List isDate;

  /// Text of text and error cells, `null` otherwise.
  final List<String?> text;

  /// Formula without `=`, `null` for cells without a formula.
  final List<String?> formula;

  /// Number formats; the `style` column indexes this list.
  final List<String> numFormat;

  Table._(this.sheetNames, this.date1904, this.sheet, this.row, this.col, this.style, this.kind, this.num, this.isDate, this.text, this.formula, this.numFormat);

  int get length => row.length;
}

/// Reads a workbook.
Table read(Uint8List data) {
  final input = _alloc(data.length);
  input.asTypedList(data.length).setAll(0, data);
  final Pointer<_Table> t;
  try {
    t = _checked((error) => _read(input, data.length, error));
  } finally {
    _dealloc(input, data.length);
  }
  try {
    final n = _size('xl_table_len')(t);
    final lenSlot = _alloc(sizeOf<Size>()).cast<Size>();
    List<String> strings(String count, String item) {
      final get = _indexed(item);
      return [
        for (var i = 0; i < _size(count)(t); i++) utf8.decode(get(t, i, lenSlot).asTypedList(lenSlot.value)),
      ];
    }

    List<String?> column(String offsets, String data, bool Function(int) present) {
      final off = _column<Uint32>(offsets)(t).asTypedList(n + 1);
      final total = off[n];
      final bytes = total == 0 ? Uint8List(0) : _column<Uint8>(data)(t).asTypedList(total);
      return [for (var i = 0; i < n; i++) present(i) ? utf8.decode(Uint8List.sublistView(bytes, off[i], off[i + 1])) : null];
    }

    Uint32List u32(String name) => n == 0 ? Uint32List(0) : Uint32List.fromList(_column<Uint32>(name)(t).asTypedList(n));
    Uint8List u8(String name) => n == 0 ? Uint8List(0) : Uint8List.fromList(_column<Uint8>(name)(t).asTypedList(n));
    try {
      final kind = u8('xl_table_kind');
      final formulaOffsets = n == 0 ? Uint32List(1) : _column<Uint32>('xl_table_formula_offsets')(t).asTypedList(n + 1);
      return Table._(
        strings('xl_table_sheet_count', 'xl_table_sheet_name'),
        _l.lookupFunction<Bool Function(Pointer<_Table>), bool Function(Pointer<_Table>)>('xl_table_date1904')(t),
        u32('xl_table_sheet'),
        u32('xl_table_row'),
        u32('xl_table_col'),
        u32('xl_table_style'),
        kind,
        n == 0 ? Float64List(0) : Float64List.fromList(_column<Double>('xl_table_num')(t).asTypedList(n)),
        u8('xl_table_is_date'),
        n == 0 ? [] : column('xl_table_text_offsets', 'xl_table_text_data', (i) => kind[i] == CellKind.text || kind[i] == CellKind.error),
        n == 0 ? [] : column('xl_table_formula_offsets', 'xl_table_formula_data', (i) => formulaOffsets[i + 1] > formulaOffsets[i]),
        strings('xl_table_num_format_count', 'xl_table_num_format'),
      );
    } finally {
      _dealloc(lenSlot.cast(), sizeOf<Size>());
    }
  } finally {
    _tableFree(t);
  }
}

/// Serial number of a date and time in the 1900 date system; `null` if not representable.
double? dateSerial(int year, int month, int day, [int hour = 0, int minute = 0, int second = 0, int microsecond = 0]) {
  final s = _dateSerial(year, month, day, hour, minute, second, microsecond);
  return s.isNaN ? null : s;
}
