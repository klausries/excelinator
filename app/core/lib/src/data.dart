part of 'app.dart';

/// The data level: the rows of a table, to filter, sort, change and export.
extension _Data on App {
  Future<void> _loadRows(bool more) async {
    if (table == null) return;
    final loaded = more ? rows.length : 0;
    final r = await _api('rows', {'store': place.store, 'region': place.item, 'view': view.spec, 'offset': loaded, 'limit': more || rows.length < 300 ? 300 : rows.length, 'summary': true});
    if (!more) {
      rows = [];
      index = [];
    }
    rows.addAll(list(r['rows']));
    index.addAll((r['index'] as List).cast<int>());
    total = r['total'] as int;
    all = r['all'] as int;
    summary = (r['summary'] as Map?)?.cast() ?? {};
    pickedOwn = null;
  }

  /// Reads the rows anew after the view changed.
  void _review(void Function() change) => _do(() async {
    change();
    await _loadRows(false);
    _changed();
  });

  Future<void> _useView(Json v) async {
    final to = RowView.of((v['view'] as Map).cast());
    if (place.item != v['region'] || place.level != Level.data) await _go(Place(place.store, Level.data, v['region'] as String));
    view = to;
    await _loadRows(false);
    _changed();
  }

  Future<Json?> _patch(Json patch, [String Function(Json)? done]) => _change('patch', {'region': place.item, 'patch': patch}, done);

  List<Part> _dataPage() {
    final t = table;
    if (t == null) return const [Parts(Lay.page, [Words('Keine Tabelle dieses Namens.')])];
    final cells = t['kind'] == 'cells';
    final stack = t['kind'] == 'stack';
    final own = list(t['columns']);
    final byName = {for (final c in own) c['name'] as String: c};
    final columns = cells
        ? <Json>[
            {'name': 'name', 'dtype': 'String'},
            {'name': 'cell', 'dtype': 'String'},
            {'name': 'value', 'dtype': ''},
          ]
        : own;
    Json? sortOf(String name) => view.sort.where((s) => s['column'] == name).firstOrNull;
    final plain = t['kind'] == 'table' || stack;
    final members = list(t['members']);

    Cell cell(int r, int k) {
      final row = rows[r], number = index[r], c = columns[k];
      final name = c['name'] as String;
      final value = row[name];
      // What the cell is of: a column of the table, or the single cell of its row.
      final Json meta = cells ? (byName[row['name']] ?? const {}) : c;
      final logic = meta['logic'] != null && (!cells || name == 'value');
      final except = cells ? null : list(meta['exceptions']).where((e) => e['row'] == number).firstOrNull;
      final fixed = cells ? name != 'value' || logic : (stack && name == 'sheet') || (logic && !(except != null && except['logic'] == null));
      return Cell(
        shown(value),
        number: value is num,
        ground: except != null ? Ground.own : !logic ? Ground.plain : meta['state'] == 'kept' ? Ground.kept : Ground.calc,
        tip: except != null ? (except['logic'] != null ? 'Eigene Formel dieser Zeile: = ${except['logic']}' : 'Fester Wert in einer berechneten Spalte') : logic ? '= ${meta['logic']}' : null,
        tap: () {
          pickedName = cells ? row['name'] as String : name;
          pickedOwn = except;
          _changed();
        },
        raw: value == null ? '' : '$value',
        edit: fixed
            ? null
            : (text) {
                if (text == (value == null ? '' : '$value')) return;
                final dtype = meta['dtype'] as String?;
                _do(() => _patch({
                  'set': [
                    cells ? {'name': row['name'], 'value': typed(text, dtype == 'String' ? 'String' : '')} : {'row': number, 'column': name, 'value': typed(text, dtype)},
                  ],
                }));
              },
      );
    }

    return [
      Parts(Lay.bar, [
        Words(t['name'] as String, Tone.strong),
        Words('${kindOf(t['kind'])}${'${t['sheet'] ?? ''}'.isEmpty ? '' : ' · ${t['sheet']}${'${t['ref'] ?? ''}'.isEmpty ? '' : '!${t['ref']}'}'}', Tone.soft),
        Entry(view.search, (text) => _review(() => view.search = text), hint: 'Suchen …'),
        Words(total == all ? '$all Zeilen' : '$total von $all Zeilen', Tone.soft),
        const Gap(),
        if (plain)
          Act('+ Zeile', () => _do(() => _patch({
            'insert': {
              'rows': [
                stack ? {'sheet': members.last['sheet']} : <String, dynamic>{},
              ],
            },
          }, (_) => 'Zeile angefügt; ihre Werte per Doppelklick eintragen.'))),
        if (plain)
          Act('− Zeilen (${picked.length})', picked.isEmpty ? null : () => _do(() {
            final gone = picked.toList();
            picked.clear();
            return _patch({'delete': gone});
          })),
        Act('Ausschnitt merken', () => _do(() async {
          final name = await host.ask('Name des Ausschnitts (Filter und Sortierung von jetzt):');
          if (name != null && name.isNotEmpty) await _change('save_view', {'name': name, 'region': place.item, 'view': view.saved}, (_) => 'Ausschnitt »$name« gespeichert.');
        }), tip: 'Filter und Sortierung unter einem Namen merken'),
        Act('CSV', () => _do(() => _export({'what': 'csv', 'region': place.item, 'view': view.spec}, (r) => 'Geschrieben: ${r['file']}'))),
        if (!stack) Act('Parquet', () => _do(() => _export({'what': 'parquet', 'region': place.item}, (r) => 'Geschrieben: ${r['file']}')), tip: 'Die Parquet-Datei mit ihrer Notiz'),
      ]),
      _formulaLine(byName),
      DataGrid(
        cols: [
          for (final c in columns)
            () {
              final name = c['name'] as String;
              final logic = !cells && c['logic'] != null;
              final s = sortOf(name);
              return Col(
                name,
                type: '${c['dtype'] ?? ''}',
                calc: logic,
                ground: !logic ? Ground.plain : c['state'] == 'kept' ? Ground.kept : Ground.calc,
                tip: logic ? '= ${c['logic']}${c['why'] != null ? '\nBraucht die Mappe: ${c['why']}' : ''}' : 'Daten',
                mark: s == null ? '' : s['desc'] == true ? ' ▼' : ' ▲',
                sort: (add) => _review(() {
                  final next = s == null ? {'column': name, 'desc': false} : s['desc'] != true ? {'column': name, 'desc': true} : null;
                  view.sort = [...(add ? view.sort.where((x) => x['column'] != name) : const <Json>[]), ?next];
                }),
                filter: cells && name == 'value' ? null : Entry(view.filter[name] ?? '', (text) => _review(() => view.filter[name] = text), hint: 'Filter', tip: 'Text enthält …, oder =, !=, <, <=, >, >= mit einem Wert, oder »leer«'),
              );
            }(),
        ],
        rows: rows.length,
        head: (r) => RowHead('${index[r] + 1}', picked: picked.contains(index[r]), tip: plain ? 'Zeile wählen (zum Löschen)' : null, pick: () {
          picked.contains(index[r]) ? picked.remove(index[r]) : picked.add(index[r]);
          _changed();
        }),
        cell: cell,
        foot: [
          for (final c in columns)
            () {
              final s = summary[c['name']] as Map?;
              return Cell(s == null ? '' : shown(s['sum']), number: true, tip: s == null ? null : 'Anzahl ${s['count']}, Minimum ${shown(s['min'])}, Maximum ${shown(s['max'])}');
            }(),
        ],
        more: rows.length < total
            ? Act('weitere Zeilen laden (${rows.length} von $total)', () => _do(() async {
                await _loadRows(true);
                _changed();
              }))
            : null,
      ),
    ];
  }

  /// The logic of the column or cell picked, in the line above the grid; a formula
  /// typed there is its new logic (checked and worked out by the workbook's engine).
  Part _formulaLine(Map<String, Json> byName) {
    final name = pickedName;
    final meta = byName[name];
    if (name == null || meta == null) {
      return const Parts(Lay.formula, [Words('Eine Spalte oder Zelle anklicken: hier steht ihre Logik. Doppelklick auf einen Wert ändert ihn.', Tone.hint)]);
    }
    final logic = meta['logic'] as String?;
    final reads = (meta['reads'] as List?) ?? const [];
    final own = pickedOwn;
    return Parts(Lay.formula, [
      Words(name, Tone.strong),
      Entry(
        logic != null ? '= $logic' : '',
        hasBook ? (text) => _do(() => _setLogic(place.item!, name, text)) : null,
        hint: hasBook ? 'keine Logik (Daten) – eine Formel eintragen, z. B. = [@qty]*[@price]' : '',
        mono: true,
        wide: true,
        enterOnly: true,
      ),
      if (meta['state'] == 'kept') Words('braucht die Mappe', Tone.badgeWarn, meta['why'] as String?) else if (logic != null) const Words('ohne Mappe berechnet', Tone.badgeOk, 'Wird aus den Daten allein berechnet'),
      if (reads.isNotEmpty) Parts(Lay.line, [const Words('liest: ', Tone.soft), for (final r in reads) _reads('$r')]),
      if (own != null) Words(own['logic'] != null ? 'diese Zeile: = ${own['logic']}' : 'diese Zeile: fester Wert', Tone.soft),
    ]);
  }

  Future<void> _setLogic(String region, String name, String text) {
    final logic = text.trim().replaceFirst(RegExp(r'^='), '').trim();
    return _change('set_logic', {'region': region, 'name': name, if (logic.isNotEmpty) 'logic': logic});
  }

  /// What a column reads, as a link to it where it is another table.
  Part _reads(String text) {
    final other = RegExp(r'^(.+?)(\.|\[)').firstMatch(text)?[1];
    final to = tables.where((t) => t['name'] == other).firstOrNull;
    return to == null ? Words(text, Tone.read) : _to(text, Place(place.store, place.level, to['name'] as String), tone: Tone.read);
  }
}
