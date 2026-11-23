part of 'app.dart';

/// The frame around every level: the head, the list at the side, and the start.
extension _Frame on App {
  List<Part> _head() {
    final stores = list(ws?['stores']);
    final undo = (info?['undo'] as List?) ?? const [];
    return [
      const Words('excelinator', Tone.brand),
      Pick(
        [
          for (final s in stores) (s['name'] as String, (s['name'] as String).replaceFirst(RegExp(r'\.store$'), '') + (s['workbook'] == true ? '' : ' (nur Daten)')),
          // A store that is not of the workspace (named by its path, where a server works on paths).
          if (place.store != null && !stores.any((s) => s['name'] == place.store)) (place.store!, place.store!.split(RegExp(r'[/\\]')).last.replaceFirst(RegExp(r'\.store$'), '')),
        ],
        place.store,
        (store) => go(Place(store, place.level)),
        hint: 'Datensatz …',
      ),
      Act('Mappen …', () {
        home = true;
        go(const Place());
      }),
      Parts(Lay.tabs, [
        // Data and model show the same tables: the one looked at stays (the program shows all).
        for (final l in Level.values)
          Act(l.label, info == null || (l == Level.book && !hasBook) || (l == Level.program && !hasBook) ? null : () => go(Place(place.store, l, l == Level.book || l == Level.program || place.level == Level.book ? null : place.item)), on: info != null && l == place.level),
      ]),
      const Gap(),
      if (info?['pending'] == true) const Words('Mappe nicht aktuell', Tone.badgeWarn, 'Daten oder Logik wurden geändert; die Mappe ist noch auf dem alten Stand.'),
      if (hasBook) Act('Mappe rechnen', () => _do(() => _change('run', {})), tip: 'Daten und Logik in die Mappe übernehmen und alles so rechnen, wie Excel es tut'),
      if (info != null)
        Act(
          '↶ Rückgängig',
          undo.isEmpty ? null : () => _do(() => _change('undo', {}, (r) => 'Zurückgenommen: ${App.named(r['undone'])}')),
          tip: undo.isEmpty ? 'Nichts zurückzunehmen' : 'Zurücknehmen: ${App.named(undo.first)}',
        ),
      if (info != null) Act('⇩ Zurück nach Excel', hasBook ? () => _do(_toExcel) : null, tip: 'Die Mappe in ihrer ursprünglichen Form mit den Daten von jetzt'),
    ];
  }

  /// Writes what the server exports and hands it to the person.
  Future<void> _export(Json args, String Function(Json) done) async {
    final r = await _api('export', {'store': place.store, ...args});
    say(done(r));
    host.deliver(r);
  }

  Future<void> _toExcel() async {
    await _export({}, (r) => 'Geschrieben: ${r['file']}${r['run'] != null ? ' (vorher gerechnet)' : ''}');
    await _show();
  }

  List<Part> _side() {
    final views = place.level == Level.data ? ((info!['views'] as Map?) ?? const {}) : const {};
    return [
      const Words('Tabellen', Tone.heading),
      for (final i in tables) _to(i['name'] as String, Place(place.store, place.level, i['name'] as String), note: kindOf(i['kind']), on: i['name'] == place.item),
      if (views.isNotEmpty) const Words('Ausschnitte', Tone.heading),
      for (final v in views.entries) Act('${v.key}', () => _do(() => _useView((v.value as Map).cast())), link: true, note: '${v.value['region']}'),
    ];
  }

  /// The sheets of the workbook, as tabs at its foot.
  List<Act> _sheetTabs() => [
    for (final s in sheets) _to(s['name'] as String, Place(place.store, Level.book, s['name'] as String), tip: '${s['rows']} Zeilen × ${s['cols']} Spalten', on: s['name'] == place.item),
  ];

  /// The start: the Excel files of the workspace, to open or to add.
  List<Part> _startPage() {
    Future<void> open(String name) async {
      final r = await _api('open', {'file': name});
      await _api('run', {'store': r['store']});
      home = false;
      await _go(Place(r['store'] as String));
    }

    final upload = backend.upload;
    final plain = list(ws!['stores']).where((s) => s['workbook'] != true).toList();
    return [
      Parts(Lay.page, [
        const Words('Mappen im Arbeitsordner', Tone.title),
        const Words('Eine Mappe wird beim Öffnen in Tabellen (Parquet-Dateien mit ihrer Logik) überführt; die Mappe bleibt als Form erhalten, in die die Daten zurückgehen.'),
        if (upload == null && list(ws!['files']).isEmpty) const Words('Hier gibt es keinen Arbeitsordner: Mappen öffnet der Agent im Gespräch und zeigt sie dann hier.', Tone.hint),
        Listing(const [], [
          for (final f in list(ws!['files']))
            ListRow([
              Words(f['name'] as String),
              if (f['store'] != null) _to('ansehen', Place(f['store'] as String)) else const Words(''),
              Act(f['store'] != null ? 'neu überführen' : 'überführen', () => _do(() async {
                final name = f['name'] as String;
                if (f['store'] == null || await host.confirm('$name neu überführen? Der vorhandene Datensatz wird ersetzt.')) await open(name);
              })),
            ]),
        ]),
        if (upload != null)
          Act('Mappe hinzufügen …', () => _do(() async {
            final file = await host.pickWorkbook();
            if (file == null) return;
            await upload(file.$1, file.$2);
            await open(file.$1);
          })),
        if (plain.isNotEmpty)
          Parts(Lay.line, [
            const Words('Nur Daten (Parquet-Dateien mit Notizen, ohne Mappe): '),
            for (final s in plain) _to(s['name'] as String, Place(s['name'] as String)),
          ]),
      ]),
    ];
  }
}
