/// The app: a store seen on three levels (data, model, workbook) through the server's
/// interface. It holds what is shown and does what is asked; [screen] is the outcome
/// as parts, and a surface draws them again whenever [listen] says so.
library;

import 'dart:async';

import 'backend.dart';
import 'parts.dart';
import 'values.dart';

part 'book.dart';
part 'data.dart';
part 'frame.dart';
part 'model.dart';
part 'program.dart';

/// The filters, order and search of the rows shown.
class RowView {
  Map<String, String> filter = {};
  List<Json> sort = [];
  String search = '';

  RowView();

  RowView.of(Json v)
    : filter = {for (final e in ((v['filter'] as Map?) ?? const {}).entries) '${e.key}': '${e.value}'},
      sort = list(v['sort']),
      search = '${v['search'] ?? ''}';

  Json get saved => {'filter': filter, 'sort': sort, 'search': search};

  /// As the server takes it.
  Json get spec => {
    'filter': [
      for (final e in filter.entries)
        if (e.value.trim().isNotEmpty) filterOf(e.key, e.value),
    ],
    'sort': sort,
    'search': search,
  };
}

class App {
  App(this.backend, this.host);

  final Backend backend;
  final Host host;

  Place place = const Place();

  /// The start (the Excel files of the workspace) is shown, though there are stores.
  bool home = false;

  /// The workspace, the description of the store, and its sheets.
  Json? ws, info;
  List<Json> sheets = [];

  // The data level: the view, the rows loaded and their numbers in the table, the rows
  // picked, and the column or cell whose logic the formula line shows.
  RowView view = RowView();
  List<Json> rows = [];
  List<int> index = [];
  int total = 0, all = 0;
  Json summary = {};
  final picked = <int>{};
  String? pickedName;
  Json? pickedOwn;

  // The workbook level: the part of the sheet loaded, and the cell picked.
  Json? part;
  int sheetRows = 300;
  (int, int)? at;

  /// What checking the data against the workbook said.
  String report = '';

  // The program level: the port of the store, and the column or cell whose code is shown.
  Json? port;
  String? portPicked;

  int busy = 0;
  final messages = <Message>[];

  final _listeners = <void Function()>[];
  final _timers = <Timer>[];
  Future<void> _last = Future.value();
  Place? _shown;

  /// Tells the surface that the screen is another now.
  void listen(void Function() f) => _listeners.add(f);

  void _changed() {
    for (final f in _listeners) {
      f();
    }
  }

  /// Everything asked of the app so far is done.
  Future<void> get settled => _last;

  void close() {
    for (final t in _timers) {
      t.cancel();
    }
    _listeners.clear();
  }

  /// Does one thing after the other; what fails is said, not thrown.
  void _do(Future<void> Function() f) {
    _last = _last.then((_) => f()).catchError((Object e) => say('$e', error: true));
  }

  void say(String text, {bool error = false}) {
    final m = Message(text, error: error);
    messages.add(m);
    _timers.add(Timer(Duration(milliseconds: error ? 9000 : 4500), () {
      messages.remove(m);
      _changed();
    }));
    _changed();
  }

  Future<Json> _api(String op, [Json args = const {}]) async {
    busy += 1;
    _changed();
    try {
      return await backend.call(op, args);
    } finally {
      busy -= 1;
      _changed();
    }
  }

  /// A call that changes the store: its outcome is said, and everything shown is read
  /// anew. Null if it failed.
  Future<Json?> _change(String op, Json args, [String Function(Json)? done]) async {
    Json? r;
    try {
      r = await _api(op, {'store': place.store, ...args});
      say(done != null ? done(r) : outcome(r));
    } catch (e) {
      say('$e', error: true);
    }
    await _show();
    return r;
  }

  void start() => _do(_show);

  /// Goes to a place: a store, a level of it, a table or sheet.
  void go(Place to) => _do(() => _go(to));

  Future<void> _go(Place to) {
    place = to;
    home = to.store == null && home;
    return _show();
  }

  /// Reads what the place needs and shows it.
  Future<void> _show() async {
    ws = await _api('workspace');
    final stores = list(ws!['stores']);
    if (place.store == null && stores.isNotEmpty && !home) place = Place(stores.first['name'] as String);
    if (place != _shown) {
      view = RowView();
      rows = [];
      index = [];
      picked.clear();
      pickedName = pickedOwn = at = part = null;
      sheetRows = 300;
      report = '';
      port = portPicked = null;
    }
    _shown = place;
    info = place.store == null ? null : await _api('tables', {'store': place.store});
    sheets = info?['workbook'] == true ? list((await _api('sheets', {'store': place.store}))['sheets']) : [];
    // The program shows every table unless one is picked.
    if (info != null && place.item == null && place.level != Level.program) {
      final first = (place.level == Level.book ? sheets : tables).firstOrNull;
      if (first != null) {
        place = _shown = Place(place.store, place.level, first['name'] as String);
      }
    }
    if (info != null && place.level == Level.data) await _loadRows(false);
    if (info != null && place.level == Level.book) await _loadSheet();
    if (info != null && place.level == Level.program) await _loadPort();
    _changed();
  }

  /// The tables of the store and its stacks (a stack has the columns of its first
  /// sheet, with the sheet in front).
  List<Json> get tables {
    if (info == null) return [];
    final real = list(info!['tables']);
    return [
      ...real,
      for (final s in list(info!['stacks']))
        {
          'name': s['name'],
          'kind': 'stack',
          'sheet': list(s['members']).map((m) => m['sheet']).join(', '),
          'ref': '',
          'members': s['members'],
          'differs': s['differs'],
          'columns': [
            {'name': 'sheet', 'dtype': 'String'},
            ...list(real.where((t) => t['name'] == list(s['members']).first['region']).firstOrNull?['columns']),
          ],
        },
    ];
  }

  Json? get table => tables.where((t) => t['name'] == place.item).firstOrNull;

  bool get hasBook => info?['workbook'] == true;

  /// What a change did, in the words of the app.
  String outcome(Json r) {
    final then = r['then'] is Map ? r['then'] as Json : r;
    if (then['recalculate'] case final Json c) {
      final kept = (c['kept'] as List).length;
      return 'Neu berechnet: ${c['done']} Spalten und Zellen.${kept > 0 ? ' $kept brauchen die Mappe (»Mappe rechnen«).' : ''}';
    }
    if (then['run'] case final Json run) {
      final open = list(run['not_computed']).fold<num>(0, (n, x) => n + (x['cells'] as num));
      final gone = (run['regions_gone'] as List? ?? const []);
      final cut = gone.isEmpty ? '' : ' Achtung: Das Blatt wird jetzt anders gelesen (nicht mehr da: ${gone.join(', ')}); »Rückgängig« stellt den Stand davor wieder her.';
      return 'Mappe gerechnet: ${run['computed']} von ${run['formula_cells']} Formelzellen.${open > 0 ? ' $open nicht berechnet.' : ''}$cut';
    }
    return 'Erledigt.';
  }

  /// What a change is called, as the server names it, in the words of the app.
  static String named(Object? what) {
    const words = {'rows of ': 'Zeilen von ', 'logic of ': 'Logik von ', 'cells of ': 'Zellen in ', 'sheet ': 'Blatt ', 'name of ': 'Name von '};
    final text = '${what ?? ''}';
    if (text == 'run') return 'Mappe rechnen';
    if (text == 'recalculate') return 'Neu berechnen';
    for (final e in words.entries) {
      if (text.startsWith(e.key)) return e.value + text.substring(e.key.length);
    }
    return text;
  }

  /// A link to a place of the app.
  Act _to(String label, Place to, {String? note, String? tip, bool on = false, Tone tone = Tone.plain}) => Act(label, () => go(to), link: true, href: to.hash, note: note, tip: tip, on: on, tone: tone);

  Screen get screen => Screen(
    head: _head(),
    side: info == null || place.level == Level.book ? const [] : _side(),
    main: info == null
        ? (ws == null ? const [] : _startPage())
        : switch (place.level) {
            Level.data => _dataPage(),
            Level.model => _modelPage(),
            Level.book => _bookPage(),
            Level.program => _programPage(),
          },
    foot: info == null || place.level != Level.book ? const [] : _sheetTabs(),
    messages: List.of(messages),
    busy: busy > 0,
  );
}
