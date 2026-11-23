part of 'app.dart';

const _regionTone = {'table': Tone.table, 'cells': Tone.cells, 'block': Tone.block};

/// The workbook level: a sheet as the workbook has it, with the regions found in it.
extension _Book on App {
  Json? get _sheet => sheets.where((s) => s['name'] == place.item).firstOrNull;

  /// How much of the sheet is loaded: (rows, columns).
  (int, int) get _size {
    final s = _sheet!;
    final rows = (s['rows'] as int) + 5, cols = (s['cols'] as int) + 2;
    return (rows < sheetRows ? rows : sheetRows, cols < 100 ? cols : 100);
  }

  Future<void> _loadSheet() async {
    if (_sheet == null) return;
    part = await _api('cells', {'store': place.store, 'sheet': place.item, 'top': 0, 'left': 0, 'rows': _size.$1, 'cols': _size.$2});
  }

  List<Part> _bookPage() {
    final sheet = _sheet, part = this.part;
    if (sheet == null || part == null) return const [Parts(Lay.page, [Words('Kein Blatt dieses Namens.')])];
    final (rows, cols) = _size;
    final cellAt = {for (final c in (part['cells'] as List).cast<List>()) (c[0] as int, c[1] as int): c};
    final styles = (part['styles'] as Map).cast<String, dynamic>();
    final regions = [
      for (final r in list(sheet['regions']))
        if (rangeOf('${r['ref']}') case final box?) (r, box),
    ];
    final covered = <(int, int), (int, int)>{};
    final spans = <(int, int), (int, int)>{};
    for (final m in (part['merges'] as List).cast<List>()) {
      final [r0 as int, c0 as int, r1 as int, c1 as int] = m;
      spans[(r0, c0)] = (r1 - r0 + 1, c1 - c0 + 1);
      for (var r = r0; r <= r1; r++) {
        for (var c = c0; c <= c1; c++) {
          if (r != r0 || c != c0) covered[(r, c)] = (r0, c0);
        }
      }
    }
    final widths = <int, double>{};
    for (final w in (part['widths'] as List).cast<List>()) {
      for (var c = w[0] as int; c <= (w[1] as int); c++) {
        widths[c] = ((w[2] as num) * 7 + 5).roundToDouble();
      }
    }
    (Json, (int, int, int, int))? regionAt(int r, int c) => regions.where((x) => r >= x.$2.$1 && r <= x.$2.$3 && c >= x.$2.$2 && c <= x.$2.$4).firstOrNull;

    final at = this.at;
    final pickedCell = at == null ? null : cellAt[at];
    final pickedRegion = at == null ? null : regionAt(at.$1, at.$2)?.$1;
    final address = at == null ? '' : '${colName(at.$2)}${at.$1 + 1}';
    return [
      Parts(Lay.bar, [
        Words(sheet['name'] as String, Tone.strong),
        Words('${sheet['rows']} Zeilen × ${sheet['cols']} Spalten', Tone.soft),
        Parts(Lay.line, [
          for (final (r, _) in regions) _to(r['name'] as String, Place(place.store, Level.data, r['name'] as String), tone: _regionTone[r['kind']] ?? Tone.plain, tip: '${kindOf(r['kind'])} ${r['ref']} – in den Daten ansehen'),
        ]),
      ]),
      Parts(Lay.formula, [
        // The name box and ƒx stand there always, as in Excel.
        Words(address, Tone.strong),
        const Words('ƒx', Tone.soft),
        if (at == null)
          const Words('Eine Zelle anklicken: hier steht ihr Wert oder ihre Formel und lässt sich ändern. Farbige Rahmen sind die erkannten Tabellen und Einzelzellen.', Tone.hint)
        else ...[
          Entry(
            pickedCell == null ? '' : '${pickedCell[3] ?? pickedCell[2]}',
            (text) => _do(() => _change('set_cells', {
              'sheet': sheet['name'],
              'cells': [
                {'cell': address, 'value': text.startsWith('=') ? text : typed(text)},
              ],
            })),
            hint: 'Wert oder =Formel, dann Enter',
            mono: true,
            wide: true,
            enterOnly: true,
          ),
          if (pickedRegion != null) Parts(Lay.line, [Words('${kindOf(pickedRegion['kind'])} ', Tone.soft), _to(pickedRegion['name'] as String, Place(place.store, Level.data, pickedRegion['name'] as String))]),
        ],
      ]),
      SheetGrid(
        rows: rows,
        cols: cols,
        colName: colName,
        width: (c) => widths[c],
        anchor: (r, c) => covered[(r, c)],
        at: at,
        cell: (r, c) {
          if (covered.containsKey((r, c))) return null;
          final cell = cellAt[(r, c)];
          final Map style = cell == null ? const {} : (styles['${cell[4]}'] as Map?) ?? const {};
          final region = regionAt(r, c);
          final box = region?.$2;
          final text = cell == null ? '' : '${cell[2]}';
          final span = spans[(r, c)];
          return SheetCell(
            text,
            bold: style['bold'] == true,
            italic: style['italic'] == true,
            fill: style['fill'] as String?,
            color: style['color'] as String?,
            align: (style['align'] as String?) ?? (RegExp(r'^-?[\d.,]+%?$').hasMatch(text) ? 'right' : null),
            rowSpan: span?.$1 ?? 1,
            colSpan: span?.$2 ?? 1,
            region: region == null ? null : _regionTone[region.$1['kind']],
            edges: box == null ? '' : '${r == box.$1 ? 't' : ''}${r == box.$3 ? 'b' : ''}${c == box.$2 ? 'l' : ''}${c == box.$4 ? 'r' : ''}',
            calc: cell != null && cell[3] != null,
            picked: at == (r, c),
            tip: region == null ? null : '${kindOf(region.$1['kind'])} ${region.$1['name']}',
            tap: () {
              this.at = (r, c);
              _changed();
            },
          );
        },
        more: (sheet['rows'] as int) + 5 > rows
            ? Act('weitere Zeilen laden ($rows von ${sheet['rows']})', () => _do(() async {
                sheetRows = rows + 600;
                await _loadSheet();
                _changed();
              }))
            : null,
        note: (sheet['cols'] as int) + 2 > cols ? 'Gezeigt sind die ersten $cols von ${sheet['cols']} Spalten.' : null,
      ),
    ];
  }
}
