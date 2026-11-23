// The desktop app from end to end: the real window, the engine built into the program,
// a folder of its own. What the core's tests read as parts, this test reads as widgets.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:xl_app/main.dart';
import 'package:xl_app/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => await RustLib.init());

  /// The app on a new folder that holds a workbook, with ways to wait and to keep
  /// pictures of the window (where `XL_SHOTS` names a folder for them).
  Future<({Directory workspace, Future<void> Function(String) until, Future<void> Function(String) shot})> opened(WidgetTester tester, String workbook) async {
    final workspace = Directory.systemTemp.createTempSync('xl-desktop-');
    File(workbook).copySync('${workspace.path}/${workbook.split('/').last}');
    final window = GlobalKey();
    await tester.pumpWidget(RepaintBoundary(key: window, child: Desktop(workspace: workspace.path, key: ValueKey(workspace.path))));

    Future<void> until(String text) async {
      for (var i = 0; i < 400 && find.text(text).evaluate().isEmpty; i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
        await tester.pump();
      }
      expect(find.text(text), findsWidgets);
    }

    Future<void> shot(String name) async {
      final to = Platform.environment['XL_SHOTS'];
      if (to == null) return;
      await tester.pump(const Duration(milliseconds: 600));
      await tester.runAsync(() async {
        final image = await (window.currentContext!.findRenderObject() as RenderRepaintBoundary).toImage(pixelRatio: 1.5);
        final data = await image.toByteData(format: ui.ImageByteFormat.png);
        File('$to/$name.png').writeAsBytesSync(data!.buffer.asUint8List());
      });
    }

    await until('Mappen im Arbeitsordner');
    await tester.tap(find.text('überführen'));
    return (workspace: workspace, until: until, shot: shot);
  }

  testWidgets('a workbook is moved into data, changed, and shown on every level', (tester) async {
    final (:workspace, :until, :shot) = await opened(tester, '${Directory.current.path}/../core/test/data/model.xlsx');
    await until('ƒ amount');
    expect(find.text('2.38'), findsOneWidget);
    expect(find.text('115.66'), findsOneWidget, reason: 'the sum of the column, at the bottom');
    expect(find.text('Einzelzellen'), findsOneWidget);

    // A value typed into a cell: two clicks open it, Enter sends it.
    final qty = Offset(tester.getCenter(find.text('qty')).dx, tester.getCenter(find.text('2.38')).dy);
    await tester.tapAt(qty);
    await tester.pump(const Duration(milliseconds: 60));
    await tester.tapAt(qty);
    await tester.pump();
    await tester.enterText(find.byType(TextField).last, '10');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await until('23.8');
    await until('Mappe nicht aktuell');

    // The logic of the column in the line above the grid.
    await tester.tap(find.text('23.8'));
    await until('ohne Mappe berechnet');
    await shot('data');

    await tester.tap(find.text('Modell'));
    await until('Stand der Überführung');
    expect(find.textContaining('2 von 3 berechneten Spalten und Zellen'), findsOneWidget);
    await shot('model');

    await tester.tap(find.text('Mappe'));
    await until('5 Zeilen × 4 Spalten');
    expect(find.text('amount'), findsOneWidget);
    // A cell picked: its address in the name box, its formula beside ƒx.
    await tester.tap(find.text('amount'));
    await until('D1');
    expect(find.text('ƒx'), findsOneWidget);
    await shot('book');
    // The sheets are tabs at the foot.
    expect(find.text('Sales'), findsWidgets);
    await tester.tap(find.text('Model').last);
    await until('3 Zeilen × 2 Spalten');

    await tester.tap(find.text('⇩ Zurück nach Excel'));
    await until('Geschrieben: model.new.xlsx (vorher gerechnet)');
    expect(File('${workspace.path}/model.new.xlsx').existsSync(), isTrue);
    workspace.deleteSync(recursive: true);
  });

  testWidgets('sheets built alike are one table, and a merged cell is one cell', (tester) async {
    final (:workspace, :until, :shot) = await opened(tester, '${Directory.current.path}/../core/test/data/months.xlsx');
    await until('Stapel');
    await tester.tap(find.text('stack'));
    await until('9 Zeilen');
    expect(find.text('Volume plan'), findsOneWidget);
    await tester.tap(find.text('Mappe'));
    await until('5 Zeilen × 4 Spalten');
    // The heading over two columns is as wide as both.
    expect(tester.getSize(find.text('Volume')).width > 0 && tester.getRect(find.ancestor(of: find.text('Volume'), matching: find.byType(Container)).first).width > 150, isTrue);
    await shot('months-book');
    workspace.deleteSync(recursive: true);
  });

  // A workbook as people built it (colours, merged cells, sheets built alike); it is
  // not part of the repository, so the test runs where it is there.
  final enron = File('${Directory.current.path}/../../out/app-test/enron_purchases.xls');
  testWidgets('a workbook of the corpus is shown with its look', skip: !enron.existsSync(), (tester) async {
    final (:workspace, :until, :shot) = await opened(tester, enron.path);
    await until('Enron Purchases_2');
    await tester.tap(find.text('Enron').last);
    await until('62 Zeilen');
    await shot('enron-data');
    await tester.tap(find.text('Mappe'));
    await until('38 Zeilen × 13 Spalten');
    expect(find.text('MW VOLUME'), findsWidgets);
    await shot('enron-book');
    await tester.tap(find.text('Modell'));
    await until('Stand der Überführung');
    await shot('enron-model');
    workspace.deleteSync(recursive: true);
  });
}
