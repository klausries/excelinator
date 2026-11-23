# Die App

Eine App, drei Oberflächen (Handbuch, Abschnitt 4):

- `core/` – was die App zeigt und tut, einmal für alle: Schnittstelle zum Server, Zustand, Bildschirme als
  Bausteine, Texte, Farben (reines Dart). Neue Funktionen gehören hierher.
- `web/` – zeichnet die Bausteine als HTML (Jaspr): für den Browser und für den Chat (MCP App).
  `tools/build_app.sh` baut sie und legt sie in `crates/xl-mcp/web`.
- `desktop/` – zeichnet die Bausteine als Widgets (Flutter): Windows und macOS, die Engine ist eingebaut.

Tests (brauchen `cargo build --profile quick -p xl-mcp`):

- `cd core && dart test` – Abläufe ohne Oberfläche gegen den echten Server.
- `cd web && dart test -p chrome` – die Seite in Chrome ohne Bildschirm, bedient wie von einem Menschen.
- `cd desktop && flutter test integration_test/flow_test.dart -d macos` – das echte Fenster (`XL_SHOTS=<Ordner>` legt
  Bilder ab).

Die Desktop-App wird nur für Apple Silicon gebaut (`ARCHS = arm64` in `desktop/macos`).
