part of 'app.dart';

/// The model level: how far the workbook is moved, and the logic of each table.
extension _Model on App {
  List<Part> _modelPage() {
    final real = list(info!['tables']);
    final t = table;
    bool calc(Json c) => c['logic'] != null;
    bool kept(Json c) => c['state'] == 'kept';
    int count(Json x, bool Function(Json) f) => list(x['columns']).where(f).length;
    final calcs = real.fold(0, (n, x) => n + count(x, calc)), kepts = real.fold(0, (n, x) => n + count(x, kept));
    final why = <String, int>{};
    for (final c in real.expand((x) => list(x['columns'])).where(kept)) {
      final w = '${c['why'] ?? ''}'.split(':').first;
      why[w] = (why[w] ?? 0) + 1;
    }
    final reasons = (why.entries.toList()..sort((a, b) => b.value - a.value)).map((e) => '${e.value} × ${e.key}').join(', ');
    final stacks = list(info!['stacks']);
    return [
      Parts(Lay.page, [
        const Words('Stand der Überführung', Tone.title),
        Words(
          (calcs > 0 ? '${calcs - kepts} von $calcs berechneten Spalten und Zellen werden aus den Daten allein berechnet (${(100 * (calcs - kepts) / calcs).round()} %).' : 'Diese Mappe enthält keine Berechnungen.') +
              (kepts > 0 ? ' Der Rest braucht die Mappe: $reasons.' : ''),
        ),
        Parts(Lay.line, [
          if (hasBook)
            Act('Daten gegen die Mappe prüfen', () => _do(() async {
              report = (await _api('verify', {'store': place.store}))['text'] as String;
              _changed();
            }))
          else
            const Words('Dieser Datensatz hat keine Mappe: Logik lässt sich hier nur ansehen.'),
          if (hasBook && stacks.isNotEmpty) Act('Blatt hinzufügen …', () => _do(() => _addSheet(list(stacks.first['members']).last['sheet'] as String))),
        ]),
        if (report.isNotEmpty) Words(report, Tone.report),
        Listing(
          const ['Tabelle', 'Art', 'Ort in der Mappe', 'Spalten / Zellen', 'berechnet', 'davon nur mit Mappe'],
          [
            for (final x in tables)
              ListRow([
                _to(x['name'] as String, Place(place.store, Level.model, x['name'] as String)),
                Words(kindOf(x['kind'])),
                Words('${x['sheet']}${'${x['ref'] ?? ''}'.isEmpty ? '' : '!${x['ref']}'}'),
                Words('${list(x['columns']).length}'),
                Words('${count(x, calc)}'),
                Words('${count(x, kept)}'),
              ], on: x['name'] == place.item),
          ],
        ),
        if (t != null) ..._detail(t),
      ]),
    ];
  }

  /// A table of the model: its columns or cells with their logic, to change.
  List<Part> _detail(Json t) {
    final stack = t['kind'] == 'stack';
    final name = t['name'] as String;
    final members = list(t['members']);
    return [
      Parts(Lay.line, [
        Words('$name – ${kindOf(t['kind'])}', Tone.heading),
        if (stack) Words('gleich gebaute Blätter: ${t['sheet']}', Tone.soft),
        if (hasBook && !stack) Act('umbenennen …', () => _do(() => _rename(name)), tip: 'Der Region einen Namen geben, der sagt, was sie ist. Die Mappe selbst ändert sich dadurch nicht.'),
      ]),
      Listing(
        [t['kind'] == 'cells' ? 'Zelle' : 'Spalte', 'Typ', 'Logik', 'Stand', 'liest', 'Ausnahmen'],
        [
          for (final c in list(t['columns']))
            () {
              final logic = c['logic'] as String?;
              final differs = list(t['differs']).where((d) => d['column'] == c['name']).firstOrNull;
              final exceptions = list(c['exceptions']);
              final shownLogic = logic != null ? '= $logic' : '';
              return ListRow([
                Parts(Lay.line, [Words(c['name'] as String), if (c['cell'] != null) Words('${c['cell']}', Tone.soft)]),
                Words('${c['dtype'] ?? ''}'),
                Parts(Lay.lines, [
                  if (!hasBook || (stack && c['name'] == 'sheet')) Words(shownLogic, Tone.mono) else Entry(shownLogic, (text) => text == shownLogic ? null : _do(() => _setLogic(name, c['name'] as String, text)), hint: 'Daten', mono: true, wide: true),
                  if (differs != null)
                    Words('nicht in jedem Blatt gleich: ${[for (final (i, l) in (differs['logic'] as List).indexed) '${members[i]['sheet']}: ${l != null ? '= $l' : 'Daten'}'].join(' · ')}', Tone.warn),
                ]),
                if (logic == null) const Words('Daten') else if (c['state'] == 'kept') Words('braucht die Mappe', Tone.badgeWarn, c['why'] as String?) else const Words('ohne Mappe', Tone.badgeOk),
                Parts(Lay.line, [for (final r in (c['reads'] as List?) ?? const []) _reads('$r')]),
                Parts(Lay.lines, [
                  for (final e in exceptions.take(8)) Words('Zeile ${(e['row'] as int) + 1}: ${e['logic'] != null ? '= ${e['logic']}' : 'fester Wert'}'),
                  if (exceptions.length > 8) Words('… und ${exceptions.length - 8} weitere'),
                ]),
              ]);
            }(),
        ],
        wide: true,
      ),
    ];
  }

  Future<void> _rename(String region) async {
    final name = await host.ask('Neuer Name für »$region«:', initial: region);
    if (name == null || name.isEmpty || name == region) return;
    final r = await _change('rename', {'region': region, 'name': name}, (_) => '»$region« heißt jetzt »$name«.');
    if (r != null) await _go(Place(place.store, Level.model, name));
  }

  Future<void> _addSheet(String last) async {
    final like = await host.ask('Welches Blatt soll kopiert werden?', initial: last);
    if (like == null || like.isEmpty) return;
    final name = await host.ask('Name des neuen Blatts:');
    if (name == null || name.isEmpty) return;
    await _change('add_sheet', {'like': like, 'name': name}, (r) => 'Blatt »$name« angelegt: ${(r['regions'] as List).join(', ')}.');
  }
}
