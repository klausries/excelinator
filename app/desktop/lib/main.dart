/// The app on a desktop: the engine is part of the program, and it works on a folder
/// of this machine.
library;

import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:material_ui/material_ui.dart';
import 'package:xl_app_core/xl_app_core.dart';

import 'host.dart';
import 'src/rust/frb_generated.dart';
import 'surface.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(const Desktop());
}

/// Where the folder worked on last is kept.
File get _kept {
  final home = Platform.isWindows ? '${Platform.environment['APPDATA']}\\excelinator' : '${Platform.environment['HOME']}/Library/Application Support/excelinator';
  return File('$home${Platform.pathSeparator}workspace.txt');
}

class Desktop extends StatefulWidget {
  /// With a [workspace] (or `XL_WORKSPACE` in the environment) the app works there;
  /// without, on the folder of last time, or it asks for one.
  const Desktop({this.workspace, super.key});
  final String? workspace;

  @override
  State<Desktop> createState() => _DesktopState();
}

class _DesktopState extends State<Desktop> {
  final navigator = GlobalKey<NavigatorState>();
  String? workspace;
  App? app;

  @override
  void initState() {
    super.initState();
    final last = widget.workspace ?? Platform.environment['XL_WORKSPACE'] ?? (_kept.existsSync() ? _kept.readAsStringSync().trim() : null);
    if (last != null && Directory(last).existsSync()) _use(last, keep: false);
  }

  void _use(String folder, {bool keep = true}) {
    if (keep) {
      _kept.parent.createSync(recursive: true);
      _kept.writeAsStringSync(folder);
    }
    app?.close();
    setState(() {
      workspace = folder;
      app = App(EngineBackend(folder), DesktopHost(navigator, folder))..start();
    });
  }

  Future<void> _choose() async {
    final folder = await getDirectoryPath(confirmButtonText: 'Hier arbeiten');
    if (folder != null) _use(folder);
  }

  ThemeData _theme(Brightness brightness) {
    final c = brightness == Brightness.dark ? dark : light;
    return ThemeData(
      brightness: brightness,
      colorScheme: ColorScheme.fromSeed(seedColor: hex(c['accent']!), brightness: brightness, primary: hex(c['accent']!), surface: hex(c['bg']!), onSurface: hex(c['text']!), outline: hex(c['line']!)),
      textTheme: Typography.material2021(platform: TargetPlatform.macOS).englishLike.apply(fontSizeFactor: 1).merge(const TextTheme(bodyMedium: TextStyle(fontSize: 13), bodyLarge: TextStyle(fontSize: 13), labelLarge: TextStyle(fontSize: 13))),
    );
  }

  @override
  Widget build(BuildContext context) {
    final folder = Tooltip(
      message: workspace ?? '',
      child: TextButton(onPressed: _choose, child: const Text('Ordner …')),
    );
    return MaterialApp(
      title: 'excelinator',
      debugShowCheckedModeBanner: false,
      navigatorKey: navigator,
      theme: _theme(Brightness.light),
      darkTheme: _theme(Brightness.dark),
      home: app == null
          ? Scaffold(
              body: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Text('excelinator', style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700)),
                    const SizedBox(height: 10),
                    const Text('Ein Ordner dieses Rechners hält die Mappen und ihre Datensätze.'),
                    const SizedBox(height: 16),
                    FilledButton(onPressed: _choose, child: const Text('Arbeitsordner wählen …')),
                  ],
                ),
              ),
            )
          : Surface(app!, extra: [folder], key: ValueKey(workspace)),
    );
  }
}
