/// The server for the tests of the page, on the side of the Dart VM: a workspace of
/// its own with the test workbooks, and the calls of the page passed on to the server.
library;

import 'dart:convert';
import 'dart:io';

import 'package:stream_channel/stream_channel.dart';
import 'package:xl_app_core/xl_app_core.dart';

void hybridMain(StreamChannel<Object?> channel, Object? message) async {
  final root = Directory.current.path.replaceFirst(RegExp(r'/app/web$'), '');
  final program = Platform.environment['XL_MCP'] ?? '$root/target/quick/xl-mcp';
  final workspace = await Directory.systemTemp.createTemp('xl-web-');
  for (final name in (message as List).cast<String>()) {
    File('$root/app/core/test/data/$name').copySync('${workspace.path}/$name');
  }
  // The server lives as long as the tests: a shell waits for its input, which ends
  // when the process of the tests ends (the isolate of this file is ended without a
  // word), and then stops the server and takes the workspace away.
  final server = await Process.start('sh', ['-c', r'"$0" --app "$1" --port 0 & p=$!; read _; kill $p; rm -rf "$1"', program, workspace.path]);
  final line = await server.stdout.transform(utf8.decoder).transform(const LineSplitter()).firstWhere((l) => l.startsWith('listening on '));
  final backend = HttpBackend(Uri.parse('${line.substring('listening on '.length)}/'));

  channel.stream.listen(
    (m) async {
      final [id as int, op as String, args as Map] = m as List;
      try {
        channel.sink.add([id, jsonEncode(await backend.call(op, args.cast())), null]);
      } on Failure catch (e) {
        channel.sink.add([id, null, e.message]);
      }
    },
    onDone: () => server.stdin.close(),
  );
  channel.sink.add('ready');
}
