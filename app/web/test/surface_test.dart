/// The page as a person uses it, in a browser without a screen (`dart test -p chrome`):
/// the surface of `lib/surface.dart` drawn by Jaspr, the app of the core, a real server
/// on a workspace of its own. What the core's tests check of the logic is not checked
/// again; here it is what is drawn and what pressing and typing on it does.
@TestOn('browser')
library;

import 'package:jaspr/client.dart';
import 'package:stream_channel/stream_channel.dart';
import 'package:test/test.dart';
import 'package:web/web.dart' as web;
import 'package:xl_app_web/host.dart';
import 'package:xl_app_web/surface.dart';
import 'package:xl_app_core/xl_app_core.dart';

import 'page.dart';

void main() {
  late StreamChannel<Object?> server;
  late Stream<Object?> words;
  late App app;
  final opened = <String>[];

  setUpAll(() async {
    server = spawnHybridUri('server.dart', message: ['model.xlsx']);
    final ready = StreamChannelController<Object?>();
    // The first word of the VM is that the server is ready; the rest are answers.
    words = server.stream.asBroadcastStream();
    await words.first;
    words.listen(ready.foreign.sink.add);
    ready.foreign.stream.listen(server.sink.add);
    app = App(ChannelBackend(ready.local), PageHost(opened.add));
    runApp(Surface(app, app.host as PageHost));
    app.start();
  });

  tearDownAll(() => server.sink.close());

  test('the start lists the workbooks, and one is opened', () async {
    await until(() => has('table.plain td', 'model.xlsx'), what: 'the start');
    expect(textOf('#status'), 'Bereit');
    expect(has('header .title .brand', 'excelinator'), isTrue);
    press(the('table.plain button', 'überführen'));
    await until(() => has('#grid td[data-at="0,1"]'), what: 'the table');
    expect(the('nav .item.on').textContent, contains('Sales'));
    expect(the('.tabs button.on').textContent, 'Daten');
    expect([for (final th in all('#grid thead tr:first-child th span')) th.textContent], ['item', 'qty', 'price', 'ƒ amount']);
    expect((cell(0, 1).textContent, cell(0, 3).textContent), ('1', '2.38'));
  });

  test('a double click on a value puts a field in its place; Enter takes it', () async {
    twice(cell(0, 1));
    await until(() => has('#grid td input.edit'), what: 'the field');
    typeIn(the('#grid td input.edit'), '7');
    await until(() => cell(0, 3).textContent == '16.66', what: 'the amount anew');
    expect(cell(0, 1).textContent, '7');
    expect(has('#grid td input.edit'), isFalse);
    expect(has('.badge.warn', 'Mappe nicht aktuell'), isTrue);
  });

  test('Escape leaves a value as it was', () async {
    twice(cell(0, 2));
    await until(() => has('#grid td input.edit'), what: 'the field');
    final field = the('#grid td input.edit') as web.HTMLInputElement;
    field.value = '99';
    field.dispatchEvent(web.Event('input', web.EventInit(bubbles: true)));
    field.dispatchEvent(web.KeyboardEvent('keydown', web.KeyboardEventInit(key: 'Escape', bubbles: true)));
    await until(() => !has('#grid td input.edit'), what: 'the field gone');
    await app.settled;
    expect(cell(0, 2).textContent, '2');
    expect(the('header button', 'Rückgängig').title, contains('Zeilen von Sales'), reason: 'the last change is still the one typed before');
  });

  test('a filter keeps the rows that match', () async {
    typeIn(all('#grid input.filter')[0], 'item1');
    await until(() => all('#grid tbody tr').length == 1, what: 'one row');
    expect(cell(0, 0).textContent, 'item1');
    typeIn(all('#grid input.filter')[0], '');
    await until(() => all('#grid tbody tr').length == 4, what: 'all rows');
  });

  test('undo takes the change back', () async {
    press(the('header button', 'Rückgängig'));
    await until(() => cell(0, 1).textContent == '1', what: 'the old value');
    expect(cell(0, 3).textContent, '2.38');
  });

  test('the workbook level is a sheet with a name box, ƒx and the sheets as tabs at the foot', () async {
    press(the('.tabs button', 'Mappe'));
    await until(() => has('#grid.sheet'), what: 'the sheet');
    expect([for (final t in all('#sheets .tab')) t.textContent], ['Sales', 'Model']);
    expect(the('#sheets .tab.on').textContent, 'Sales');
    expect(has('nav .item'), isFalse, reason: 'no list at the side: the sheets are below');
    press(cell(1, 3));
    await until(() => textOf('.formula > b') == 'D2', what: 'the name box');
    expect(textOf('.formula > small'), 'ƒx');
    expect((the('.formula input') as web.HTMLInputElement).value, startsWith('=ROUND('));
    expect(textOf('#grid thead th.at'), 'D', reason: 'the column of the picked cell is marked');
    expect(textOf('#grid tbody th.at'), '2');
    expect(cell(1, 3).classList.contains('picked'), isTrue);

    press(cell(1, 1));
    await until(() => textOf('.formula > b') == 'B2');
    typeIn(the('.formula input'), '3');
    await until(() => cell(1, 3).textContent == '7.14', what: 'the formula reckoned anew');

    press(the('#sheets .tab', 'Model'));
    await until(() => the('#sheets .tab.on').textContent == 'Model', what: 'the other sheet');
  });

  test('the model level renames a table through a dialog on the page', () async {
    press(the('.tabs button', 'Modell'));
    await until(() => has('h2', 'Stand der Überführung'), what: 'the model');
    press(the('nav .item', 'Sales'));
    await until(() => has('button', 'umbenennen'));
    press(the('button', 'umbenennen'));
    await until(() => has('.dialog input'), what: 'the dialog');
    typeIn(the('.dialog input'), 'Verkauf');
    await until(() => has('nav .item', 'Verkauf'), what: 'the new name');
    expect(has('.modal'), isFalse);
  });

  test('a question can be declined', () async {
    press(the('header .title button', 'Mappen'));
    await until(() => has('table.plain button', 'neu überführen'), what: 'the start');
    press(the('table.plain button', 'neu überführen'));
    await until(() => has('.dialog'), what: 'the question');
    press(the('.dialog button', 'Abbrechen'));
    await until(() => !has('.modal'));
    expect(has('table.plain button', 'neu überführen'), isTrue);
  });
}
