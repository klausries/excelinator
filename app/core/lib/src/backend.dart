import 'dart:convert';

import 'package:http/http.dart' as http;

typedef Json = Map<String, dynamic>;

/// What a call of the interface says when it fails, in the server's words.
class Failure implements Exception {
  Failure(this.message);
  final String message;
  @override
  String toString() => message;
}

/// The interface of the server (see `crates/xl-mcp/src/api.rs`): the name of a call and
/// its arguments, the answer as JSON. How the call travels is the backend's business:
/// over HTTP, through a chat's host, or straight into the engine built into a program.
abstract class Backend {
  Future<Json> call(String op, [Json args = const {}]);

  /// Puts an Excel file into the workspace; null where that cannot be done.
  Future<void> Function(String name, List<int> bytes)? get upload => null;
}

/// The server over HTTP: `POST <base>api/<token>/<call>`.
class HttpBackend extends Backend {
  HttpBackend(this.base, {this.token = 'local', http.Client? client}) : _client = client ?? http.Client();

  /// The address of the server, ending in `/`.
  final Uri base;
  final String token;
  final http.Client _client;

  @override
  Future<Json> call(String op, [Json args = const {}]) async {
    final r = await _client.post(base.resolve('api/$token/$op'), body: jsonEncode(args));
    final answer = jsonDecode(utf8.decode(r.bodyBytes));
    if (r.statusCode != 200) throw Failure(answer is Map ? '${answer['error']}' : 'HTTP ${r.statusCode}');
    return answer as Json;
  }

  @override
  Future<void> Function(String, List<int>) get upload => (name, bytes) async {
    final r = await _client.put(base.resolve('files/$token/${Uri.encodeComponent(name)}'), body: bytes);
    if (r.statusCode != 200) throw Failure(r.body);
  };
}

/// What the app needs of the surface it runs on, besides drawing.
abstract class Host {
  /// A text asked of the person; null if they would rather not.
  Future<String?> ask(String question, {String initial = ''});

  Future<bool> confirm(String question);

  /// An Excel file the person chose: its name and content.
  Future<(String, List<int>)?> pickWorkbook();

  /// What an export wrote (`file`, and `url` where the server gives one): a browser
  /// fetches it, a desktop shows where it is.
  void deliver(Json written);
}
