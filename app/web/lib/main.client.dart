/// The app as a page. Served by the server (`/app/<token>/`) it talks to that server;
/// shown by a chat in a frame (an MCP App) it talks through the chat's host.
library;

import 'dart:convert';
import 'dart:js_interop';

import 'package:jaspr/client.dart';
import 'package:web/web.dart' as web;
import 'package:xl_app_core/xl_app_core.dart';

import 'host.dart';
import 'surface.dart';

@JS('JSON.stringify')
external JSString _stringify(JSAny? value);

@JS('JSON.parse')
external JSAny? _parse(JSString text);

final _colours = web.HTMLStyleElement();

void main() {
  web.document.head!.append(_colours..textContent = cssVariables());
  final token = RegExp(r'/app/([^/]+)/').firstMatch(web.window.location.pathname)?[1];
  token == null ? _inChat() : _inBrowser(token);
}

void _inBrowser(String token) {
  final at = web.window.location;
  final host = PageHost((url) => at.href = url);
  final app = App(HttpBackend(Uri.parse('${at.origin}/'), token: token), host);

  // The place of the app is the address of the page, both ways. Only a place the app
  // came to itself is written into the address (not one it is still on its way to); if
  // it only says more than the address (the first table of a store) it takes the
  // address's place in the history, any other is a step one can go back from.
  var written = at.hash;
  app.listen(() {
    final hash = app.place.hash;
    if (hash == written || app.busy > 0) return;
    written = hash;
    if (at.hash == hash || (at.hash.isEmpty && hash == '#/')) return;
    hash.startsWith(at.hash) && at.hash.length > 2 ? web.window.history.replaceState(null, '', hash) : at.hash = hash;
  });
  web.window.addEventListener('hashchange', ((web.Event _) {
    if (at.hash == written) return;
    written = at.hash;
    app.go(Place.parse(at.hash));
  }).toJS);

  app.place = Place.parse(at.hash);
  app.start();
  runApp(Surface(app, host));
}

void _inChat() {
  late final ChatBackend chat;
  chat = ChatBackend((m) => web.window.parent?.postMessage(_parse(jsonEncode(m).toJS), '*'.toJS));
  final host = PageHost((url) => chat.request('ui/open-link', {'url': url}));
  final app = App(chat, host);
  web.window.addEventListener('message', ((web.MessageEvent e) {
    final m = jsonDecode(_stringify(e.data).toDart);
    if (m is Map && m['jsonrpc'] == '2.0') chat.receive(m.cast());
  }).toJS);

  // The look and the room are the host's: its theme, and a height it can give.
  void fit() {
    _colours.textContent = cssVariables(chat.context['theme'] as String?);
    final room = (chat.context['containerDimensions'] as Map?) ?? const {};
    final most = (room['maxHeight'] as num?) ?? 640;
    final height = (room['height'] as num?) ?? (most < 640 ? most : 640);
    web.document.documentElement!.setAttribute('style', 'height:${height}px');
    chat.notify('ui/notifications/size-changed', {'width': web.window.innerWidth, 'height': height});
  }

  chat.onContext = fit;
  // The call that showed the app says where it starts.
  chat.onInput = (a) => app.go(Place(a['store'] as String?, Level.values.where((l) => l.name == a['level']).firstOrNull ?? Level.data, a['item'] as String?));
  runApp(Surface(app, host));
  chat.connect().then((_) {
    fit();
    app.start();
  });
}
