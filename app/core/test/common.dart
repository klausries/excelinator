/// What the tests of the app share: a server of their own on a workspace of their
/// own, a host that answers as told, and ways to find parts of a screen.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:xl_app_core/xl_app_core.dart';

/// The root of the repository (the tests run in `app/core`).
final root = Directory.current.path.replaceFirst(RegExp(r'/app/core$'), '');

/// The server program: `XL_MCP`, or what `cargo build --profile quick -p xl-mcp` built.
String get program => Platform.environment['XL_MCP'] ?? '$root/target/quick/xl-mcp';

class Server {
  Server(this.process, this.workspace, this.base);
  final Process process;
  final Directory workspace;
  final Uri base;

  /// A server on a new workspace that holds a test workbook.
  static Future<Server> start([String workbook = 'model.xlsx']) async {
    final workspace = await Directory.systemTemp.createTemp('xl-app-');
    File('$root/app/core/test/data/$workbook').copySync('${workspace.path}/$workbook');
    final process = await Process.start(program, ['--app', workspace.path, '--port', '0']);
    final line = await process.stdout.transform(utf8.decoder).transform(const LineSplitter()).firstWhere((l) => l.startsWith('listening on '));
    return Server(process, workspace, Uri.parse('${line.substring('listening on '.length)}/'));
  }

  void stop() {
    process.kill();
    workspace.deleteSync(recursive: true);
  }
}

/// The server over stdin and stdout, as a chat on this machine starts it: it works on
/// paths, without a workspace.
class Stdio {
  Stdio(this.process) : _lines = StreamIterator(process.stdout.transform(utf8.decoder).transform(const LineSplitter()));
  final Process process;
  final StreamIterator<String> _lines;
  var _id = 0;
  Future<void> _last = Future.value();

  static Future<Stdio> start() async => Stdio(await Process.start(program, []));

  /// One request and its answer; one after the other.
  Future<Json> mcp(String method, Json params) {
    final answer = _last.then((_) async {
      process.stdin.writeln(jsonEncode({'jsonrpc': '2.0', 'id': ++_id, 'method': method, 'params': params}));
      await _lines.moveNext();
      return jsonDecode(_lines.current) as Json;
    });
    _last = answer;
    return answer;
  }
}

/// A person who answers what they were told to, in order, and keeps what they got.
class TestHost implements Host {
  final answers = <String?>[];
  final delivered = <Json>[];
  (String, List<int>)? file;

  @override
  Future<String?> ask(String question, {String initial = ''}) async => answers.removeAt(0);
  @override
  Future<bool> confirm(String question) async => true;
  @override
  Future<(String, List<int>)?> pickWorkbook() async => file;
  @override
  void deliver(Json written) => delivered.add(written);
}

/// Every part of a list of parts, those inside others too.
Iterable<Part> every(Iterable<Part> parts) sync* {
  for (final p in parts) {
    yield p;
    if (p is Parts) yield* every(p.kids);
    if (p is Listing) yield* every(p.rows.expand((r) => r.cells));
  }
}

extension Find on Screen {
  Iterable<Part> get parts => every([...head, ...side, ...main, ...foot]);

  /// The button or link with a label (or the start of one).
  Act act(String label) {
    final acts = parts.whereType<Act>().toList();
    return acts.where((a) => a.label == label).firstOrNull ?? acts.firstWhere((a) => a.label.startsWith(label), orElse: () => throw StateError('no act "$label" among ${acts.map((a) => a.label).toList()}'));
  }

  bool has(String label) => parts.whereType<Act>().any((a) => a.label.startsWith(label));

  /// Everything written on the screen outside the grids.
  String get words => parts.whereType<Words>().map((w) => w.text).join('\n');

  DataGrid get grid => parts.whereType<DataGrid>().single;
  SheetGrid get sheet => parts.whereType<SheetGrid>().single;

  /// The line that takes logic or a cell's content.
  Entry get formula => every(main.whereType<Parts>().where((p) => p.lay == Lay.formula)).whereType<Entry>().single;
}

extension Read on DataGrid {
  int col(String name) => cols.indexWhere((c) => c.name == name);
  Cell at(int row, String name) => cell(row, col(name));
  List<String> column(String name) => [for (var r = 0; r < rows; r++) at(r, name).text];
}
