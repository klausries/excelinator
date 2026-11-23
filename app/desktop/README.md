# excelinator auf dem Desktop

Die App für Windows und macOS (Flutter). Sie zeichnet die Bildschirme von `app/core` als
Widgets; die Engine ist über `flutter_rust_bridge` eingebaut (`rust/`, ruft
`xl_mcp::app_call`), es läuft kein Server.

    flutter run -d macos                                   # starten
    flutter test integration_test/flow_test.dart -d macos  # Ablauftest mit echter Engine
    flutter_rust_bridge_codegen generate                   # nach Änderungen in rust/src/api
