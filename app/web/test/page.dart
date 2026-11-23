/// What the tests of the page share: the app on a server of its own (through the test's
/// channel to the Dart VM, see `server.dart`), and ways to find, press and type on the
/// page as a person would.
library;

import 'dart:async';
import 'dart:convert';
import 'package:stream_channel/stream_channel.dart';
import 'package:test/test.dart';
import 'package:web/web.dart' as web;
import 'package:xl_app_core/xl_app_core.dart';

/// The server's interface, through the channel to the VM.
class ChannelBackend extends Backend {
  ChannelBackend(this.channel) {
    channel.stream.listen((m) {
      if (m case [final int id, final answer, final error]) {
        final done = _waiting.remove(id)!;
        error == null ? done.complete(jsonDecode(answer as String) as Json) : done.completeError(Failure('$error'));
      }
    });
  }
  final StreamChannel<Object?> channel;
  final _waiting = <int, Completer<Json>>{};
  var _id = 0;

  @override
  Future<Json> call(String op, [Json args = const {}]) {
    final done = _waiting[++_id] = Completer<Json>();
    channel.sink.add([_id, op, args]);
    return done.future;
  }
}

/// Waits until [ok] holds (the app has answered and the page is drawn anew).
Future<void> until(bool Function() ok, {String? what}) async {
  for (var i = 0; i < 300; i++) {
    if (ok()) return;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  fail('waited in vain${what == null ? '' : ' for $what'}; the page says: ${web.document.body?.textContent}');
}

List<web.HTMLElement> all(String selector) {
  final list = web.document.querySelectorAll(selector);
  return [for (var i = 0; i < list.length; i++) list.item(i)! as web.HTMLElement];
}

/// The one element of [selector] whose text is [text] (or that holds it).
web.HTMLElement the(String selector, [String? text]) {
  final found = all(selector).where((e) => text == null || e.textContent!.trim() == text).toList();
  if (found.isEmpty && text != null) found.addAll(all(selector).where((e) => e.textContent!.contains(text)));
  expect(found, isNotEmpty, reason: 'no $selector ${text ?? ''}');
  return found.first;
}

bool has(String selector, [String? text]) => all(selector).any((e) => text == null || e.textContent!.contains(text));

String textOf(String selector) => the(selector).textContent!.trim();

void press(web.HTMLElement e) => e.click();

void twice(web.HTMLElement e) => e.dispatchEvent(web.MouseEvent('dblclick', web.MouseEventInit(bubbles: true)));

/// Types into a field and finishes with Enter.
void typeIn(web.HTMLElement field, String text) {
  final input = field as web.HTMLInputElement;
  input.value = text;
  input.dispatchEvent(web.Event('input', web.EventInit(bubbles: true)));
  input.dispatchEvent(web.KeyboardEvent('keydown', web.KeyboardEventInit(key: 'Enter', bubbles: true)));
}

/// A cell of the grid on the page, by row and column (`data-at`).
web.HTMLElement cell(int row, int col) => the('#grid td[data-at="$row,$col"]');
