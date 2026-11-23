# Ein Release machen (Vorabversion)

## Was ein Release ist

| Datei | Inhalt | Für wen |
|---|---|---|
| `xlcells-VERSION-py3-none-any.whl` | das Python-Paket; sein Rechenkern ist ein WebAssembly-Modul (`xlcells/_wasm/xlcells.wasm`, aus `bindings/wasm`), läuft auf jedem Prozessor mit Python ≥ 3.11, Polars und `wasmtime` | alle mit Python (`uv tool install` / `uv pip install`) |
| `excelinator-VERSION-windows-x64.zip`, `…-arm64.zip` | ein Ordner zum Entpacken: eingebettetes Python 3.13.17, das Paket mit Polars 2.1.0 und wasmtime 51.0.0 entpackt, uv 0.13.9, `.cmd`-Dateien | Windows-Rechner, auch stark gesicherte: keine Installation, keine Administratorrechte, nichts zu bauen, kein Netz |
| `excel-port-skill.zip` | die Anleitung für Agenten zur Portierung (`skills/excel-port`) als Skill für Claude | wer mit Claude portiert |
| `SHA256SUMS` | Prüfsummen | alle |

Die Version steht in `packaging/VERSION` (jetzt `0.1.0.dev1`: Vorabversion vor der
ersten Alpha). Die Rust-Crates und die Desktop-App gehören nicht zu diesem Release.

## Schritte

1. Auf dem Mac prüfen: `uv run python tools/build_release.py` baut alles nach `dist/`;
   dann das Rad in eine leere Umgebung installieren und prüfen:

       uv venv target/release-check --python 3.12
       uv pip install --python target/release-check dist/xlcells-*.whl
       target/release-check/bin/python tools/smoke_release.py --mcp target/release-check/bin/xlcells-mcp

2. Den öffentlichen Stand erzeugen (ein neues Repository mit einem Commit, ohne die
   Geschichte; geprüft auf große Dateien, Pfade dieses Rechners, Rechnernamen, Schlüssel):

       PUBLIC_NAME="…" PUBLIC_EMAIL="…@users.noreply.github.com" tools/make_public.sh

   Er liegt dann in `out/public/excelinator`. Die Adresse `…@users.noreply.github.com`
   hält die eigene E-Mail-Adresse aus dem öffentlichen Commit.

3. Auf GitHub ein Repository anlegen (zuerst privat, wenn man erst sehen will, ob der
   Ablauf durchläuft), dann aus `out/public/excelinator`:

       git remote add origin https://github.com/KONTO/excelinator.git
       git push -u origin main
       git tag v0.1.0.dev1 && git push origin v0.1.0.dev1

4. Der Ablauf `release` (`.github/workflows/release.yml`) baut Rad und Windows-Ordner,
   prüft den Windows-Ordner auf Windows x64 und ARM64 (kompiliert und im Interpreter),
   das Rad mit uv auf Windows, Linux x64 und ARM64 und macOS, und legt bei Erfolg ein
   GitHub-Release als Vorabversion mit allen Dateien an. `ci` prüft jeden Push (Rust-Tests,
   alle Python-Tests gegen das Rad).

5. PyPI ist nicht Teil des Ablaufs. Wer `uv tool install xlcells` ohne Dateinamen will,
   veröffentlicht das Rad dort selbst (Trusted Publishing einrichten, dann
   `uv publish dist/xlcells-*.whl`).

## Was nicht hochgeladen wird

| Was | Warum |
|---|---|
| die bisherige Git-Geschichte | Frühere Commits enthalten Korpus-Dateien fremder Herkunft (umgewandelte Enron-Mappen, Testdateien von LibreOffice und Apache POI), und die Autorenangaben tragen die Namen der eigenen Rechner (`…fritz.box`, `….local`). Der öffentliche Stand beginnt deshalb mit einem neuen Commit (`tools/make_public.sh`). |
| `corpus/` außer `manifest.tsv` | 6,5 GB fremde Dateien mit eigenen Lizenzen, darunter echte Enron-Mappen; `tools/fetch_corpus.sh` holt sie. Ohne Korpus überspringen die Tests, die ihn brauchen. |
| `out/` | Ergebnisse von Läufen auf diesem Rechner: Testordner, Protokolle von Claude Desktop und Claude Code, Bilder, exportierte Mappen. |
| `target/`, `dist/`, `.venv/`, `app/*/build/` | Gebautes; das Release baut GitHub selbst. |
| `bindings/python/python/xlcells/_wasm/xlcells.wasm` | gebaut (`tools/build_wasm.sh`), steckt im Rad. |
| Python, uv, Polars, wasmtime für Windows | holt `tools/build_release.py` beim Bauen von python.org, GitHub und PyPI (festgeschriebene Versionen, Räder per SHA-256 geprüft). |
| Notizen und Pläne der Arbeit mit Claude | liegen außerhalb des Repositorys. |

Was im öffentlichen Stand bleibt und man kennen sollte: `corpus/manifest.tsv` (Namen,
Prüfsummen, Herkunft und Lizenz der Korpus-Dateien, keine Inhalte); in zwei Tests steht
der Vorname »Klaus« als Autor eines Kommentars; die gebaute Web-App
(`crates/xl-mcp/web/main.client.dart.js`, damit der Server ohne Dart baut); die
Lizenzen MIT und Apache-2.0 nennen Klaus Ries als Inhaber (ohne E-Mail-Adresse).

## Was geprüft ist und was nicht

Geprüft auf diesem Mac (Apple Silicon): alle 201 Python-Tests gegen das installierte Rad
in einer leeren Umgebung (Python 3.12, der Rechenkern aus dem wasm-Modul; 12 übersprungen:
Korpus, Schemas, LibreOffice, native Engines), die Tests des Servers und der App gegen den
Server auf dem wasm-Modul, der Rauchtest (auch die App mit gehaltener Verbindung), die
Umrechnung von Windows-Pfaden als Unit-Test, der Interpreter von wasmtime; die Portierung
mit einem Agenten (Claude Code, nur Polars): Enron-Mappe 32 von 32 eigenständig, 0
abweichend, auch auf veränderten Daten gleich dem Rechenkern.

Nicht geprüft, bevor GitHub den Ablauf `release` laufen lässt: Windows überhaupt (der
Ordner, die `.cmd`-Dateien, Laufwerke und Pfade), Linux und ARM-Linux. Danach nicht
geprüft: Claude Desktop unter Windows, Rechner mit Richtlinien gegen ausführbaren
Speicher (der Rückfall auf den Interpreter ist nur hier erzwungen geprüft), Netzlaufwerke
mit `\\server\freigabe`-Pfaden (werden nicht umgesetzt), Microsoft Copilot.

Bekannte Grenzen der Vorabversion: Der erste Start übersetzt das Modul (etwa 2 s, danach
aus dem Cache von wasmtime im Benutzerordner); im Interpreter rechnet es etwa viermal
langsamer. Der Ordner bringt die übliche Laufzeit von Polars (`polars-runtime-32`);
für sehr alte Prozessoren, denen Befehle fehlen, die sie voraussetzt, gibt es
`polars-runtime-compat` (nicht geprüft, nicht im Ordner).
