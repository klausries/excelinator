import 'dart:async';
import 'dart:js_interop';

import 'package:web/web.dart' as web;
import 'package:xl_app_core/xl_app_core.dart';

/// Something the page asks the person: a text to type ([input]), or yes or no.
class Question {
  Question(this.text, this.initial, this.input);
  final String text;
  final String initial;
  final bool input;
  final done = Completer<String?>();
}

/// The page as the app's host. It asks in a dialog of its own (a frame in a chat may
/// not open the browser's), takes files through the browser's chooser, and hands what
/// was written to [open] (the browser fetches it; a chat's host opens the link).
class PageHost implements Host {
  PageHost(this.open);
  final void Function(String url) open;

  /// The question that waits for its answer, and who is told when there is one.
  Question? question;
  void Function()? changed;

  Future<String?> _put(Question q) {
    question = q;
    changed?.call();
    return q.done.future;
  }

  /// The person's answer; null if they would rather not.
  void answer(String? text) {
    final q = question;
    question = null;
    changed?.call();
    q?.done.complete(text);
  }

  @override
  Future<String?> ask(String question, {String initial = ''}) => _put(Question(question, initial, true));

  @override
  Future<bool> confirm(String question) async => await _put(Question(question, '', false)) != null;

  @override
  Future<(String, List<int>)?> pickWorkbook() {
    final done = Completer<(String, List<int>)?>();
    final input = web.HTMLInputElement()
      ..type = 'file'
      ..accept = '.xlsx,.xlsm,.xls';
    input.onchange = ((web.Event _) {
      final file = input.files?.item(0);
      if (file == null) return done.complete(null);
      file.arrayBuffer().toDart.then((b) => done.complete((file.name, b.toDart.asUint8List())), onError: done.completeError);
    }).toJS;
    input.addEventListener('cancel', ((web.Event _) => done.complete(null)).toJS);
    input.click();
    return done.future;
  }

  @override
  void deliver(Json written) {
    if (written['url'] case final String url) open(url);
  }
}
