part of 'app.dart';

const _how = {'polars': 'lesbar', 'plan': 'Übersetzung', 'rest': 'Rest'};
const _state = {'ok': 'richtig', 'differs': 'weicht ab', 'kept': 'nicht berechnet', 'of the moment': 'zeitabhängig'};

/// The program level: the workbook as a Python program on Polars (a port), how far it
/// is, and the code of each calculated column and cell.
extension _Program on App {
  Future<void> _loadPort() async {
    port = await _api('port', {'store': place.store});
  }

  Future<void> _makePort(bool again) async {
    if (again && !await host.confirm('Das Programm neu schreiben? Was darin von Hand geändert wurde, geht verloren.')) return;
    port = await _api('export_port', {'store': place.store, 'replace': again});
    say('Programm geschrieben: ${port!['folder']}');
    _changed();
  }

  List<Part> _programPage() {
    final p = port;
    if (p == null) return const [];
    if (p['exists'] != true) {
      return [
        Parts(Lay.page, [
          const Words('Programm', Tone.title),
          const Words('Die Mappe als Python-Programm auf Polars: es rechnet, was die Mappe rechnet, wird Wert für Wert gegen sie geprüft und läuft ohne Excel und ohne xlcells. Was sich noch nicht übersetzen lässt, ist als Rest benannt.'),
          Words('Ordner: ${p['folder']}'),
          if (hasBook) Act('Programm erzeugen', () => _do(() => _makePort(false))) else const Words('Dieser Datensatz hat keine Mappe: ein Programm wird aus einer Mappe erzeugt.', Tone.hint),
        ]),
      ];
    }
    final items = list(p['items']);
    int count(String key, String value) => items.where((i) => i[key] == value).length;
    final standalone = count('how', 'polars') + count('how', 'plan');
    final shown = place.item == null ? items : items.where((i) => i['region'] == place.item).toList();
    final key = portPicked;
    final picked = items.where((i) => '${i['region']}/${i['name']}' == key).firstOrNull;
    final checked = p['checked'] == true;
    return [
      Parts(Lay.page, [
        const Words('Programm', Tone.title),
        Words(
          'Berechnete Spalten und Zellen: ${items.length}. Mit Polars allein: $standalone (${count('how', 'polars')} lesbar, ${count('how', 'plan')} als Übersetzung des Rechenkerns). Noch nicht übersetzt (Rest): ${count('how', 'rest')}.'
          '${checked ? ' Letzte Prüfung: richtig ${count('state', 'ok')}, abweichend ${count('state', 'differs')}, nicht berechnet ${count('state', 'kept')}.' : ''}',
        ),
        Words('Ordner: ${p['folder']}'),
        Parts(Lay.line, [
          if (p['can_check'] == true)
            Act('Prüfen', () => _do(() async {
              port = await _api('check_port', {'store': place.store});
              say('Geprüft: ${count('state', 'differs') == 0 ? 'keine Abweichung' : '${count('state', 'differs')} weichen ab'}.');
              _changed();
            }), tip: 'check.py des Programms ausführen: jeden Wert gegen die Mappe'),
          Act(p['url'] != null ? 'Herunterladen' : 'Ordner zeigen', () => host.deliver({'file': p['folder'], 'url': p['url']})),
          Act('Neu erzeugen …', hasBook ? () => _do(() => _makePort(true)) : null),
          if (place.item != null) _to('alle Tabellen', Place(place.store, Level.program)),
        ]),
        const Parts(Lay.lines, [Words('Ein Rest wird in port/calc.py an seiner Zeile durch eigenen Polars-Code ersetzt; check.py sagt danach, ob die Werte stimmen. Die Anleitung für Agenten liefert das Werkzeug port_guide.', Tone.hint)]),
        Listing(
          const ['Tabelle', 'Spalte oder Zelle', 'Logik', 'Art', 'Geprüft', 'Zeile'],
          [
            for (final i in shown)
              ListRow([
                Words('${i['region']}'),
                Act('${i['name']}', () {
                  portPicked = '${i['region']}/${i['name']}';
                  _changed();
                }, link: true),
                Words('${i['logic'] ?? i['formula']}', Tone.mono),
                Words(_how[i['how']] ?? '${i['how']}', i['how'] == 'rest' ? Tone.badgeWarn : Tone.badgeOk, i['reason'] as String?),
                Words(_state[i['state']] ?? '–', i['state'] == 'differs' ? Tone.warn : Tone.plain),
                Words('${i['line']}'),
              ], on: '${i['region']}/${i['name']}' == key),
          ],
          wide: true,
        ),
        if (picked != null) ...[
          Words('${picked['region']} · ${picked['name']}: port/calc.py, Zeile ${picked['line']}', Tone.heading),
          if (picked['reason'] != null) Words('${picked['reason']}', Tone.hint),
          Words('${picked['code']}', Tone.report),
        ],
      ]),
    ];
  }
}
