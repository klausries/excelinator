import 'dart:convert';
import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:material_ui/material_ui.dart';
import 'package:xl_app_core/xl_app_core.dart';

import 'src/rust/api/simple.dart' as engine;
import 'surface.dart';

/// The engine built into the program, on a folder of this machine.
class EngineBackend extends Backend {
  EngineBackend(this.workspace);
  final String workspace;

  @override
  Future<Json> call(String op, [Json args = const {}]) async {
    try {
      return jsonDecode(await engine.call(workspace: workspace, op: op, args: jsonEncode(args))) as Json;
    } catch (e) {
      throw Failure('$e');
    }
  }

  @override
  Future<void> Function(String, List<int>) get upload =>
      (name, bytes) => File('$workspace${Platform.pathSeparator}$name').writeAsBytes(bytes);
}

/// The desktop as the app's host: dialogs of its own, the file chooser of the system,
/// and what was written shown in its folder.
class DesktopHost implements Host {
  DesktopHost(this.navigator, this.workspace);
  final GlobalKey<NavigatorState> navigator;
  final String workspace;

  @override
  Future<String?> ask(String question, {String initial = ''}) {
    final text = TextEditingController(text: initial);
    final focus = FocusNode();
    selectAllOnFocus(focus, text);
    return showDialog<String>(
      context: navigator.currentContext!,
      builder: (context) => AlertDialog(
        content: SizedBox(
          width: 380,
          child: TextField(controller: text, focusNode: focus, autofocus: true, decoration: InputDecoration(labelText: question), onSubmitted: (t) => Navigator.pop(context, t)),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Abbrechen')),
          FilledButton(onPressed: () => Navigator.pop(context, text.text), child: const Text('OK')),
        ],
      ),
    );
  }

  @override
  Future<bool> confirm(String question) async =>
      await showDialog<bool>(
        context: navigator.currentContext!,
        builder: (context) => AlertDialog(
          content: Text(question),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Abbrechen')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('OK')),
          ],
        ),
      ) ??
      false;

  @override
  Future<(String, List<int>)?> pickWorkbook() async {
    final file = await openFile(
      acceptedTypeGroups: const [
        XTypeGroup(label: 'Excel', extensions: ['xlsx', 'xlsm', 'xls']),
      ],
    );
    return file == null ? null : (file.name, await file.readAsBytes());
  }

  @override
  void deliver(Json written) {
    final path = '$workspace${Platform.pathSeparator}${written['file']}';
    if (Platform.isMacOS) Process.run('open', ['-R', path]);
    if (Platform.isWindows) Process.run('explorer', ['/select,', path]);
  }
}
