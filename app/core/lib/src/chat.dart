import 'dart:async';
import 'dart:convert';

import 'backend.dart';

/// The app inside a chat (an MCP App): it has no way to the server but through the
/// chat's host. Calls travel as JSON-RPC messages: [send] hands one to the host
/// (`postMessage` in a frame), [receive] takes what the host says. Each call of the
/// interface is the server's tool `app_call`.
class ChatBackend extends Backend {
  ChatBackend(this.send);
  final void Function(Json message) send;

  /// What the host says of itself: theme, size, how the app is shown.
  Json context = {};

  /// The arguments of the tool call that showed the app (`store`, `level`, `item`).
  void Function(Json arguments)? onInput;
  void Function()? onContext;

  var _id = 0;
  final _open = <int, Completer<Json>>{};

  Future<Json> request(String method, [Json params = const {}]) {
    final id = ++_id;
    // The answer may come before `send` is back.
    final answer = _open[id] = Completer<Json>();
    send({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params});
    return answer.future;
  }

  void notify(String method, [Json params = const {}]) => send({'jsonrpc': '2.0', 'method': method, 'params': params});

  /// Says hello to the host; after this it may send what the app was shown for.
  Future<void> connect() async {
    final r = await request('ui/initialize', {
      'protocolVersion': '2026-01-26',
      'appInfo': {'name': 'excelinator', 'version': '0.1.0'},
      'appCapabilities': {
        'availableDisplayModes': ['inline', 'fullscreen'],
      },
    });
    context = (r['hostContext'] as Map?)?.cast() ?? {};
    notify('ui/notifications/initialized');
  }

  /// A message of the host: the answer to a request, or something it tells or asks.
  void receive(Json m) {
    final params = (m['params'] as Map?)?.cast<String, dynamic>() ?? {};
    if (m['method'] == null) {
      final open = _open.remove(m['id']);
      if (m['error'] case final Map e) {
        open?.completeError(Failure('${e['message']}'));
      } else {
        open?.complete((m['result'] as Map?)?.cast() ?? {});
      }
    } else if (m['method'] == 'ui/notifications/tool-input') {
      onInput?.call((params['arguments'] as Map?)?.cast() ?? {});
    } else if (m['method'] == 'ui/notifications/host-context-changed') {
      context = {...context, ...params};
      onContext?.call();
    } else if (m['id'] != null) {
      // What the host asks (that the app is about to go, or whether it is there).
      send({'jsonrpc': '2.0', 'id': m['id'], 'result': <String, dynamic>{}});
    }
  }

  @override
  Future<Json> call(String op, [Json args = const {}]) async {
    final r = await request('tools/call', {
      'name': 'app_call',
      'arguments': {'op': op, 'args': args},
    });
    final text = [for (final c in (r['content'] as List? ?? const [])) '${c['text'] ?? ''}'].join();
    if (r['isError'] == true) throw Failure(text);
    return jsonDecode(text) as Json;
  }
}
