// The app from end to end, without a surface: a real server, the app's own calls, and
// the screens it makes. What a surface draws is what these tests read.
import 'dart:convert';
import 'dart:io';

import 'package:test/test.dart';
import 'package:xl_app_core/xl_app_core.dart';

import 'common.dart';

void main() {
  chatTests();
  moreTests();
  late Server server;
  late TestHost host;
  late App app;

  /// Presses, types or picks, and waits for the app to be done with it.
  Future<void> after(void Function() what) async {
    what();
    await app.settled;
  }

  setUp(() async {
    server = await Server.start();
    host = TestHost();
    app = App(HttpBackend(server.base), host);
    await after(app.start);
  });

  tearDown(() {
    app.close();
    server.stop();
  });

  /// Moves the test workbook into a store, as a person does from the start.
  Future<void> open() => after(() => app.screen.act('überführen').run!());

  test('the program level makes the workbook a Python program and shows how far it is', () async {
    await open();
    await after(() => app.screen.act('Programm').run!());
    expect(app.place.item, isNull, reason: 'the program shows every table');
    expect(app.screen.words, contains('Die Mappe als Python-Programm auf Polars'));
    await after(() => app.screen.act('Programm erzeugen').run!());
    expect(app.screen.words, contains('Berechnete Spalten und Zellen: 3. Mit Polars allein: 2'));
    expect(app.screen.words, contains('Ordner: model-port'));
    // A column picked shows its lines of the program.
    await after(() => app.screen.act('amount').run!());
    final code = app.screen.parts.whereType<Words>().firstWhere((w) => w.tone == Tone.report).text;
    expect(code, allOf(contains('# Sales[amount]'), contains('pl.col("qty")')));
    // Checked where the server has Python (xlcells-mcp); the program xl-mcp has none.
    if (app.screen.has('Prüfen')) {
      await after(() => app.screen.act('Prüfen').run!());
      expect(app.screen.words, contains('Letzte Prüfung: richtig 2, abweichend 0'));
    }
    // Made anew only when the person says so.
    await after(() => app.screen.act('Neu erzeugen').run!());
    expect(app.screen.words, contains('Mit Polars allein: 2'));
    // The program goes out as a zip from the files of the workspace.
    await after(() => app.screen.act('Herunterladen').run!());
    expect(host.delivered.last['url'], endsWith('/model-port.zip'));
  });

  test('the start lists the workbooks; opening one shows its first table', () async {
    expect(app.screen.words, contains('Mappen im Arbeitsordner'));
    expect(app.screen.words, contains('model.xlsx'));
    expect(app.screen.side, isEmpty);
    await open();
    expect(app.place, const Place('model.xlsx.store', Level.data, 'Sales'));
    final grid = app.screen.grid;
    expect(grid.cols.map((c) => c.name), ['item', 'qty', 'price', 'amount']);
    expect(grid.cols.last.calc, isTrue);
    expect(grid.cols.last.tip, contains('ROUND([@qty]*[@price]'));
    expect(grid.column('amount'), ['2.38', '4.76', '99', '9.52']);
    expect(grid.foot.map((c) => c.text), ['', '10', '8', '115.66']);
    expect(app.screen.words, contains('4 Zeilen'));
    expect(app.screen.side.whereType<Act>().map((a) => '${a.label} ${a.note}'), ['Sales Tabelle', 'Model Einzelzellen']);
  });

  test('a value typed into a row is calculated on, and taken back', () async {
    await open();
    final grid = app.screen.grid;
    expect(grid.at(0, 'amount').edit, isNull, reason: 'a calculated cell is not typed in');
    expect(grid.at(2, 'amount').ground, Ground.own, reason: 'a fixed value in a calculated column');
    expect(grid.at(2, 'amount').edit, isNotNull);
    await after(() => grid.at(0, 'qty').edit!('10'));
    expect(app.screen.grid.at(0, 'amount').text, '23.8');
    expect(app.screen.messages.last.text, startsWith('Neu berechnet'));
    expect(app.screen.words, contains('Mappe nicht aktuell'));
    // The single cells that read the table follow.
    await after(() => app.screen.act('Model').run!());
    expect(app.screen.grid.column('value'), ['0.19', '137.08', '']);
    expect(app.screen.grid.at(1, 'value').edit, isNull);
    await after(() => app.screen.act('↶ Rückgängig').run!());
    expect(app.screen.messages.last.text, 'Zurückgenommen: Zeilen von Sales');
    expect(app.screen.grid.column('value'), ['0.19', '115.66', '']);
    expect(app.screen.act('↶ Rückgängig').tip, 'Zurücknehmen: Mappe rechnen', reason: 'what is left is the run that opened the store');
  });

  test('rows are filtered, sorted, searched, added and deleted', () async {
    await open();
    await after(() => app.screen.grid.cols[1].filter!.done!('>= 3'));
    expect(app.screen.grid.column('item'), ['item2', 'item3']);
    expect(app.screen.words, contains('2 von 4 Zeilen'));
    expect(app.screen.grid.foot[1].text, '7');
    await after(() => app.screen.grid.cols[1].sort(false));
    await after(() => app.screen.grid.cols[1].sort(false));
    expect(app.screen.grid.cols[1].mark, ' ▼');
    expect(app.screen.grid.column('item'), ['item3', 'item2']);
    expect(app.screen.grid.head(0).label, '4', reason: 'rows keep their number in the table');
    // A view of this is kept by name and comes back.
    host.answers.add('viel');
    await after(() => app.screen.act('Ausschnitt merken').run!());
    await after(() => app.screen.grid.cols[1].filter!.done!(''));
    expect(app.screen.grid.rows, 4);
    await after(() => app.screen.act('viel').run!());
    expect(app.screen.grid.column('item'), ['item3', 'item2']);
    await after(() => app.screen.grid.cols[1].filter!.done!(''));
    await after(() => app.screen.grid.cols[1].sort(false));
    // Search.
    final search = app.screen.parts.whereType<Entry>().firstWhere((e) => e.hint == 'Suchen …');
    await after(() => search.done!('item1'));
    expect(app.screen.grid.column('item'), ['item1']);
    await after(() => app.screen.parts.whereType<Entry>().firstWhere((e) => e.hint == 'Suchen …').done!(''));
    // A row more, with values typed in; then a row less.
    await after(() => app.screen.act('+ Zeile').run!());
    expect(app.screen.grid.rows, 5);
    await after(() => app.screen.grid.at(4, 'qty').edit!('2'));
    await after(() => app.screen.grid.at(4, 'price').edit!('5'));
    expect(app.screen.grid.at(4, 'amount').text, '11.9');
    expect(app.screen.act('− Zeilen').run, isNull);
    await after(() => app.screen.grid.head(1).pick!());
    expect(app.screen.grid.head(1).picked, isTrue);
    await after(() => app.screen.act('− Zeilen (1)').run!());
    expect(app.screen.grid.column('item'), ['item0', 'item2', 'item3', '']);
  });

  test('the logic of a column is shown and changed in the formula line', () async {
    await open();
    expect(app.screen.words, contains('Eine Spalte oder Zelle anklicken'));
    await after(() => app.screen.grid.at(0, 'amount').tap!());
    expect(app.screen.formula.value, startsWith('= ROUND([@qty]*[@price]'));
    expect(app.screen.words, contains('ohne Mappe berechnet'));
    await after(() => app.screen.formula.done!('= [@qty]*[@price]'));
    expect(app.screen.messages.last.text, startsWith('Mappe gerechnet'));
    expect(app.screen.grid.column('amount'), ['2', '4', '99', '8']);
    expect(app.screen.formula.value, '= [@qty]*[@price]');
    // A formula that is none is refused, and nothing changes.
    await after(() => app.screen.formula.done!('= [@nothing]+'));
    expect(app.screen.messages.last.error, isTrue);
    expect(app.screen.grid.column('amount'), ['2', '4', '99', '8']);
  });

  test('the model level says how far the workbook is moved and renames', () async {
    await open();
    await after(() => app.screen.act('Modell').run!());
    expect(app.place.level, Level.model);
    expect(app.screen.words, contains('2 von 3 berechneten Spalten und Zellen werden aus den Daten allein berechnet (67 %)'));
    expect(app.screen.words, contains('Der Rest braucht die Mappe: 1 × volatile function'));
    final listings = app.screen.parts.whereType<Listing>().toList();
    expect(listings.first.rows.map((r) => (r.cells.first as Act).label), ['Sales', 'Model']);
    expect(listings.last.rows.map((r) => every(r.cells).whereType<Words>().first.text), ['item', 'qty', 'price', 'amount']);
    await after(() => app.screen.act('Daten gegen die Mappe prüfen').run!());
    expect(app.report, contains('agree'));
    expect(app.screen.words, contains(app.report));
    host.answers.add('Verkauf');
    await after(() => app.screen.act('umbenennen').run!());
    expect(app.place, const Place('model.xlsx.store', Level.model, 'Verkauf'));
    expect(app.screen.side.whereType<Act>().map((a) => a.label), ['Verkauf', 'Model']);
    // The logic of the cells that read the table names it anew.
    await after(() => app.screen.act('Model').run!());
    expect(app.screen.parts.whereType<Entry>().map((e) => e.value), contains('= SUM(Verkauf[amount])'));
  });

  test('the workbook level shows the sheet and takes a cell', () async {
    await open();
    await after(() => app.screen.act('Mappe').run!());
    expect(app.place, const Place('model.xlsx.store', Level.book, 'Sales'));
    final sheet = app.screen.sheet;
    expect(sheet.cell(0, 3)!.text, 'amount');
    expect(sheet.cell(1, 3)!.calc, isTrue);
    expect(sheet.cell(0, 0)!.edges, 'tl');
    expect(sheet.cell(4, 3)!.edges, 'br');
    expect(sheet.cell(0, 0)!.region, Tone.table);
    expect(sheet.cell(6, 0)!.region, isNull);
    await after(() => sheet.cell(1, 1)!.tap!());
    expect(app.screen.sheet.cell(1, 1)!.picked, isTrue);
    expect(app.screen.words, contains('B2'));
    await after(() => app.screen.formula.done!('7'));
    expect(app.screen.sheet.cell(1, 3)!.text, '16.66');
    await after(() => app.screen.sheet.cell(1, 3)!.tap!());
    expect(app.screen.formula.value, startsWith('=ROUND('));
  });

  test('the workbook goes back to Excel with the data of now', () async {
    await open();
    await after(() => app.screen.grid.at(0, 'qty').edit!('10'));
    await after(() => app.screen.act('⇩ Zurück nach Excel').run!());
    expect(host.delivered.single['file'], 'model.new.xlsx');
    expect(host.delivered.single['run'], isNotNull, reason: 'the workbook is run first');
    expect(File('${server.workspace.path}/model.new.xlsx').existsSync(), isTrue);
    expect(app.screen.words, isNot(contains('Mappe nicht aktuell')));
    await after(() => app.screen.act('CSV').run!());
    expect(File('${server.workspace.path}/${host.delivered.last['file']}').readAsStringSync(), startsWith('item,qty,price,amount\nitem0,10,2,23.8\n'));
  });

  test('a workbook is added, and stores are gone between by name', () async {
    host.file = ('second.xlsx', File('${server.workspace.path}/model.xlsx').readAsBytesSync());
    await after(() => app.screen.act('Mappe hinzufügen').run!());
    expect(app.place.store, 'second.xlsx.store');
    await after(() => app.screen.act('Mappen …').run!());
    expect(app.screen.words, contains('Mappen im Arbeitsordner'));
    await open();
    final pick = app.screen.head.whereType<Pick>().single;
    expect(pick.options.map((o) => o.$2), ['model.xlsx', 'second.xlsx']);
    expect(pick.value, 'model.xlsx.store');
    await after(() => pick.pick('second.xlsx.store'));
    expect(app.place, const Place('second.xlsx.store', Level.data, 'Sales'));
  });

  test('places are written and read as addresses', () {
    for (final p in const [Place(), Place('a b.store'), Place('m.xlsx.store', Level.book, 'Blatt 1'), Place('m.store', Level.model, 'T')]) {
      expect(Place.parse(p.hash), p.store == null ? const Place() : (p.item == null ? Place(p.store, p.level) : p));
    }
    expect(const Place('m.xlsx.store', Level.book, 'Blatt 1').hash, '#/m.xlsx.store/book/Blatt%201');
    expect(Place.parse('#/'), const Place());
    expect(typed('3,5'), 3.5);
    expect(typed('7'), 7);
    expect(typed('7', 'String'), '7');
    expect(typed('wahr'), true);
    expect(typed(' '), isNull);
    expect(shown(2.0), '2');
    expect(shown(1 / 3), '0.333333');
    expect(colName(27), 'AB');
    expect(rangeOf('B2:AB10'), (1, 1, 9, 27));
  });
}

/// The app on a server of its own that holds one workbook, opened into a store.
Future<(Server, TestHost, App)> opened(String workbook) async {
  final server = await Server.start(workbook);
  final host = TestHost();
  final app = App(HttpBackend(server.base), host)..start();
  await app.settled;
  app.screen.act('überführen').run!();
  await app.settled;
  return (server, host, app);
}

void moreTests() {
  test('sheets built alike are one table, and a sheet more is added', () async {
    final (server, host, app) = await opened('months.xlsx');
    Future<void> after(void Function() what) async {
      what();
      await app.settled;
    }

    expect(app.screen.side.whereType<Act>().map((a) => '${a.label} ${a.note}'), ['Jan Tabelle', 'Feb Tabelle', 'Mar Tabelle', 'stack Stapel']);
    await after(() => app.screen.act('stack').run!());
    var grid = app.screen.grid;
    expect(grid.cols.map((c) => c.name), ['sheet', 'item', 'Volume plan', 'Volume actual', 'difference']);
    expect(grid.column('sheet'), ['Jan', 'Jan', 'Jan', 'Feb', 'Feb', 'Feb', 'Mar', 'Mar', 'Mar']);
    expect(grid.at(0, 'sheet').edit, isNull, reason: 'the sheet of a row is where it lies');
    expect(app.screen.has('Parquet'), isFalse, reason: 'a stack has no file of its own');
    // A value of the second sheet, through the stack.
    await after(() => grid.at(3, 'Volume plan').edit!('2.5'));
    expect(app.screen.grid.at(3, 'difference').text, '10');
    // A row more goes to the last sheet.
    await after(() => app.screen.act('+ Zeile').run!());
    expect(app.screen.grid.rows, 10);
    expect(app.screen.grid.at(9, 'sheet').text, 'Mar');
    await after(() => app.screen.act('↶ Rückgängig').run!());

    // The model says where the sheets differ in their logic.
    await after(() => app.screen.act('Modell').run!());
    expect(app.screen.words, contains('gleich gebaute Blätter: Jan, Feb, Mar'));
    expect(app.screen.words, contains('nicht in jedem Blatt gleich: Jan: = [@[Volume actual]]-[@[Volume plan]] · Feb: = [@[Volume actual]]-[@[Volume plan]] · Mar: = ROUND([@[Volume actual]]-[@[Volume plan]],0)'));
    expect(app.screen.has('umbenennen'), isFalse);
    host.answers.addAll(['Mar', 'Apr']);
    await after(() => app.screen.act('Blatt hinzufügen').run!());
    expect(app.screen.messages.last.text, startsWith('Blatt »Apr« angelegt'));
    await after(() => app.screen.act('Daten').run!());
    expect(app.place.item, 'stack', reason: 'the table looked at stays when the level changes');
    expect(app.screen.grid.column('sheet').toSet(), {'Jan', 'Feb', 'Mar', 'Apr'});

    // The sheet as it lies in the workbook: a merged cell covers its neighbour.
    await after(() => app.screen.act('Mappe').run!());
    expect(app.screen.foot.map((a) => a.label), ['Jan', 'Feb', 'Mar', 'Apr'], reason: 'the sheets are tabs at the foot, as in Excel');
    expect(app.screen.side, isEmpty);
    expect(app.screen.foot.where((a) => a.on).single.label, 'Jan');
    final sheet = app.screen.sheet;
    expect((sheet.cell(0, 1)!.text, sheet.cell(0, 1)!.colSpan), ('Volume', 2));
    expect(sheet.cell(0, 2), isNull);
    expect((sheet.anchor(0, 2), sheet.anchor(0, 1)), ((0, 1), null));
    expect(sheet.cell(2, 1)!.text, '10');
    app.close();
    server.stop();
  });

  test('Parquet files with their notes are data to work on without a workbook', () async {
    final (server, _, app) = await opened('model.xlsx');
    Future<void> after(void Function() what) async {
      what();
      await app.settled;
    }

    // The files alone, somewhere else.
    final plain = Directory('${server.workspace.path}/plain')..createSync();
    for (final f in Directory('${server.workspace.path}/model.xlsx.store/data').listSync().whereType<File>()) {
      f.copySync('${plain.path}/${f.uri.pathSegments.last}');
    }
    await after(() => app.screen.act('Mappen …').run!());
    expect(app.screen.words, contains('Nur Daten (Parquet-Dateien mit Notizen, ohne Mappe): '));
    await after(() => app.screen.act('plain').run!());
    expect((app.place.store, app.place.level), ('plain', Level.data));
    await after(() => app.screen.act('Sales').run!());
    expect(app.screen.head.whereType<Pick>().single.options.map((o) => o.$2), ['model.xlsx', 'plain (nur Daten)']);
    // No workbook: nothing to run or to give back, and its level cannot be gone to.
    expect(app.screen.has('Mappe rechnen'), isFalse);
    expect(app.screen.act('⇩ Zurück nach Excel').run, isNull);
    expect(app.screen.act('Mappe').run, isNull);
    // Changed and calculated again, as in a store.
    await after(() => app.screen.grid.at(0, 'qty').edit!('10'));
    expect(app.screen.grid.at(0, 'amount').text, '23.8');
    await after(() => app.screen.grid.at(0, 'amount').tap!());
    expect(app.screen.formula.done, isNull, reason: 'logic is changed with the workbook only');
    await after(() => app.screen.act('Modell').run!());
    expect(app.screen.words, contains('Dieser Datensatz hat keine Mappe: Logik lässt sich hier nur ansehen.'));
    expect(app.screen.parts.whereType<Entry>(), isEmpty);
    app.close();
    server.stop();
  });
}

/// A chat's host, as far as the app needs one: it answers the app's hello, and hands
/// its tool calls to the server (over HTTP, as a chat would over its own connection).
class TestChat {
  /// [mcp] is the chat's connection to the server: a request, its answer.
  TestChat(this.mcp);
  final Future<Json> Function(String method, Json params) mcp;
  late final backend = ChatBackend(_fromApp);
  final said = <String>[];

  /// The connection of a chat elsewhere: over HTTP, into the server's workspace.
  static Future<Json> Function(String, Json) http(Server server) {
    var id = 0;
    return (method, params) async {
      final client = HttpClient();
      final request = await client.postUrl(server.base.resolve('mcp/local'));
      request.write(jsonEncode({'jsonrpc': '2.0', 'id': ++id, 'method': method, 'params': params}));
      final answer = jsonDecode(await utf8.decodeStream(await request.close())) as Json;
      client.close();
      return answer;
    };
  }

  void _fromApp(Json m) {
    said.add('${m['method']}');
    if (m['method'] == 'ui/initialize') {
      backend.receive({
        'jsonrpc': '2.0',
        'id': m['id'],
        'result': {
          'protocolVersion': '2026-01-26',
          'hostInfo': {'name': 'test', 'version': '1'},
          'hostCapabilities': {'serverTools': {}},
          'hostContext': {'theme': 'dark'},
        },
      });
    } else if (m['method'] == 'tools/call') {
      mcp('tools/call', m['params'] as Json).then((answer) => backend.receive({...answer, 'id': m['id']}));
    }
  }
}

void chatTests() {
  test('inside a chat the app is a resource of the server and works through its host', () async {
    final server = await Server.start();
    final chat = TestChat(TestChat.http(server));
    // What a chat reads of the server: the tool that shows the app, and the app.
    final tools = ((await chat.mcp('tools/list', {}))['result']['tools'] as List).cast<Map>();
    final show = tools.firstWhere((t) => t['name'] == 'show_app');
    final uri = show['_meta']['ui']['resourceUri'];
    expect(uri, 'ui://excelinator/app.html');
    expect(tools.firstWhere((t) => t['name'] == 'app_call')['_meta']['ui']['visibility'], ['app']);
    final page = (await chat.mcp('resources/read', {'uri': uri}))['result']['contents'][0] as Map;
    expect(page['mimeType'], 'text/html;profile=mcp-app');
    expect(page['text'], allOf(startsWith('<!doctype html>'), contains('<style>'), isNot(contains('src='))));
    expect((await chat.mcp('resources/list', {}))['result']['resources'][0]['uri'], uri);

    // The agent opens the workbook and shows the app; the app starts where it was told.
    await chat.mcp('tools/call', {'name': 'open_workbook', 'arguments': {'path': 'model.xlsx'}});
    final shown = await chat.mcp('tools/call', {'name': 'show_app', 'arguments': {'store': 'model.xlsx.store', 'level': 'data', 'item': 'Sales'}});
    expect(shown['result']['content'][0]['text'], contains('The app shows model.xlsx.store'));

    final app = App(chat.backend, TestHost());
    chat.backend.onInput = (a) => app.go(Place(a['store'] as String?, Level.values.byName('${a['level'] ?? 'data'}'), a['item'] as String?));
    await chat.backend.connect();
    expect(chat.backend.context['theme'], 'dark');
    expect(chat.said, ['ui/initialize', 'ui/notifications/initialized']);
    chat.backend.receive({'jsonrpc': '2.0', 'method': 'ui/notifications/tool-input', 'params': {'arguments': {'store': 'model.xlsx.store', 'level': 'data', 'item': 'Sales'}}});
    await app.settled;
    expect(app.screen.grid.column('amount'), ['2.38', '4.76', '99', '9.52']);
    app.screen.grid.at(0, 'qty').edit!('10');
    await app.settled;
    expect(app.screen.grid.at(0, 'amount').text, '23.8');
    expect(app.screen.messages.last.text, startsWith('Neu berechnet'));
    // What fails at the server is said in the app.
    app.go(const Place('model.xlsx.store', Level.data, 'Nothing'));
    await app.settled;
    expect(app.screen.words, contains('Keine Tabelle dieses Namens.'));
    expect(app.backend.upload, isNull, reason: 'a chat has no way to put files into the workspace');
    app.close();
    server.stop();
  });

  test('a chat on this machine names the store by its path', () async {
    final folder = Directory.systemTemp.createTempSync('xl-chat-');
    File('$root/app/core/test/data/model.xlsx').copySync('${folder.path}/model.xlsx');
    final server = await Stdio.start();
    final chat = TestChat(server.mcp);
    await chat.mcp('tools/call', {'name': 'open_workbook', 'arguments': {'path': '${folder.path}/model.xlsx'}});
    final store = '${folder.path}/model.xlsx.store';

    final app = App(chat.backend, TestHost());
    chat.backend.onInput = (a) => app.go(Place(a['store'] as String?));
    await chat.backend.connect();
    chat.backend.receive({'jsonrpc': '2.0', 'method': 'ui/notifications/tool-input', 'params': {'arguments': {'store': store}}});
    await app.settled;
    expect(app.place, Place(store, Level.data, 'Sales'));
    expect(app.screen.grid.column('amount'), ['2.38', '4.76', '99', '9.52']);
    final pick = app.screen.head.whereType<Pick>().single;
    expect(pick.value, store);
    expect(pick.options, [(store, 'model.xlsx')]);
    app.screen.grid.at(0, 'qty').edit!('10');
    await app.settled;
    expect(app.screen.grid.at(0, 'amount').text, '23.8');
    // There is no workspace to choose from: the agent opens workbooks.
    app.screen.act('Mappen …').run!();
    await app.settled;
    expect(app.screen.words, contains('Hier gibt es keinen Arbeitsordner'));
    expect(app.screen.has('Mappe hinzufügen'), isFalse);
    app.close();
    server.process.kill();
    folder.deleteSync(recursive: true);
  });
}
