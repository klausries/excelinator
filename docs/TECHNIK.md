# excelinator: technische Notizen

Excel-Port und Excel-Ablösung. Rust für alles außer UI und Scripting, Dart für Scripting, Flutter für die UI.

## Installieren (Vorabversion 0.1.0.dev1)

Das Python-Paket `xlcells` trägt seinen Rechenkern als ein WebAssembly-Modul
(`bindings/wasm`, ausgeführt von `wasmtime`): ein Paket für jeden Prozessor und jedes
System, auf dem Rechner wird nichts gebaut. Es braucht Python ab 3.11, Polars und
`wasmtime` (beide als fertige Pakete für Windows, macOS und Linux).

    uv tool install xlcells-0.1.0.dev1-py3-none-any.whl     # der MCP-Server als Programm xlcells-mcp
    uv pip install xlcells-0.1.0.dev1-py3-none-any.whl      # die Bibliothek in einer Umgebung

Für Windows gibt es je einen Ordner für x64 und ARM64
(`excelinator-0.1.0.dev1-windows-x64.zip`): entpacken und benutzen, ohne Installation und
ohne Administratorrechte. Er bringt Python (eingebettete Fassung von python.org), die
Pakete und uv mit; `xlcells-mcp.cmd` startet den MCP-Server, `xlcells-app.cmd ORDNER` die
App im Browser, `README.txt` darin sagt, wie Claude Desktop ihn einträgt. Wo ein Rechner
keinen zur Laufzeit erzeugten Code erlaubt, rechnet das Modul im Interpreter von
`wasmtime` (langsamer, von selbst; `XLCELLS_WASM_TARGET=pulley64` wählt ihn fest).

Gebaut wird ein Release mit `uv run python tools/build_release.py` (Rad und
Windows-Ordner nach `dist/`) und geprüft mit `tools/smoke_release.py`; auf GitHub baut
und prüft es der Ablauf `.github/workflows/release.yml` auf Windows (x64, ARM64), Linux
und macOS. Was zu einem Release gehört und was nicht hochgeladen wird:
[RELEASE.md](RELEASE.md).

## Stand

| Paket | Zweck | Gate |
| --- | --- | --- |
| `xl-opc` | OOXML-Paket (ZIP, Content Types, Relationships) verlustfrei lesen und schreiben | Korpus: jeder Eintrag nach Lesen und Schreiben byte-identisch |
| `xl-serial` | Datums-Seriennummern in den Systemen 1900 und 1904, inklusive 29.2.1900 | Jede Seriennummer hin und zurück identisch |
| `xl-formula` | Formeln verlustfrei parsen und ausgeben (Syntaxbaum mit Operator-Rangfolge) | Jede Formel im Korpus geht Text → Baum → Text identisch |
| `xl-xls` | Excel 97–2003 (`.xls`, BIFF8) lesen: Werte, Formeln, Formate, Namen, externe Bezüge | Korpus gegen calamine, xlrd und LibreOffice |
| `xl-look` | Aussehen einer Zelle: jedes Formatattribut aufgelöst (Zellformat, Formatvorlage, Zeile/Spalte, Tabellenformat, bedingte Formatierung), mit Herkunft | Korpus gegen LibreOffice, Tabellenformate gegen ECMA-376 |
| `xl-golden` | Korpus-Manifest, Round-Trip-Prüfung, Neuberechnungs-Vergleich | Werkzeug `xlg` |

## Korpus

`tools/fetch_corpus.sh` holt die Quellen nach `corpus/` (nicht im Repository) und schreibt `corpus/manifest.tsv` mit SHA-256, Herkunft und Lizenz.

Die abgeleiteten Teile (`lo-*`) erzeugt LibreOffice. Die Werkzeuge (`tools/lo_oracle.py`, `tools/lo_look.py`) steuern es
über ein Basic-Makro in einem eigenen, englischsprachigen Profil (`tools/lo_basic.py`), nicht über LibreOffices Python:
Das ist ein eigenes Programm im Anwendungspaket, das macOS nach einer Installation per Homebrew nicht startet
(Quarantäne). Nötig ist nur `soffice`; ein Wächter gibt eine Datei nach 2 Minuten oder 1 GB Zwischendateien auf.
Mit LibreOffice 26.8 (November 2026): `lo-oracle` 458 Dateien, `lo-enron` 1.485, `lo-look` 1.480 (eine aufgegeben).
LibreOffice 26 speichert Zellen mit unbekannten Funktionen (Makrofunktionen) als `#NAME?` ohne Formel; der
`.xls`-Vergleich zählt sie gesondert.

| Quelle | Inhalt | Lizenz |
| --- | --- | --- |
| rust_xlsxwriter `tests/input` | In Excel erstellte Referenzdateien | MIT OR Apache-2.0 |
| calamine `tests` | Grenzfälle für Leser | MIT |
| Apache POI `test-data/spreadsheet` | Dateien aus Fehlerberichten, Funktionstests | Apache-2.0 |
| Enron (SheetJS `enron_xls`) | Echte Arbeitsmappen, jede 14. Datei als Stichprobe | siehe `src-enron/LICENSE` |
| `lo-oracle`, `lo-enron` | Die `.xls`-Dateien der Quellen, von LibreOffice neu berechnet und als xlsx gespeichert | abgeleitet |
| `lo-look` | Für Stichprobenzellen jeder xlsx: LibreOffices Formatierung der Zelle und die Farben seines HTML-Exports (`tools/lo_look.py`) | abgeleitet |

## Befehle

    cargo test --release                 # Unit-Tests und Korpus-Gate
    cargo run --release --bin xlg -- roundtrip   # Bericht nach out/roundtrip.tsv
    cargo run --release --bin xlg -- recalc      # IronCalc gegen gespeicherte Werte, Bericht nach out/
    cargo run --release --bin xlg -- formulas    # Parser-Gate über alle Formeln
    # Erwartete Werte aus der ursprünglichen xls (Excels gespeicherte Werte), nur Enron:
    cargo run --release --bin xlg -- recalc corpus out/enron-excel 30 legacy lo-enron

## Lese-/Schreibmodul (Arbeitsname `xlcells`)

Eigenständiges Paket: Excel-Dateien (xlsx/xlsm) lesen und schreiben, ohne Polars, ohne
Rechenkern. Lizenz MIT oder Apache-2.0; Laufzeitabhängigkeiten nur `zip` und `quick-xml`.

| Schicht | Ort | Inhalt |
| --- | --- | --- |
| Paket | `crates/xl-opc` | ZIP/OPC, verlustfreier Rundlauf |
| Zelltabelle | `crates/xl-cells` | Arbeitsmappe ↔ eine spaltenförmige Tabelle (Blatt, Zeile, Spalte, Typ, Zahl, Text, Formel, Stil) |
| pyExcelerator-Ebene | `crates/xl-compat` | `Workbook`, `add_sheet`, `write`, `write_merge`, `Formula`, `XFStyle`, `parse_xls` |
| Python | `bindings/python` | `xlcells` (Spalten), Fassaden `xlcells.openpyxl`, `xlcells.polars`, `xlcells.pyexcelerator` |
| Python, Rechenkern | `bindings/python-engine` | `xlcells-engine`: Formelberechnung, Tabellen als DataFrames, Regionen, Speicher, Aussehen von Zellen; rechnet auf dem Polars der Python-Umgebung |
| Python, Rechenkern mit Polars | `bindings/python-engine-bundled` | `xlcells-engine-bundled`: derselbe Code, mit eingebautem Polars gebaut (Referenz) |
| JavaScript/TypeScript | `bindings/js` | WebAssembly (Node und Browser), Typen inklusive |
| C | `bindings/c` | `include/xlcells.h` |
| Dart | `bindings/dart` | `dart:ffi` auf die C-Schnittstelle, ohne Laufzeitabhängigkeit |

Drei Python-Pakete, getrennt installierbar:

| Paket | Inhalt | Wheel | Polars | Python |
| --- | --- | --- | --- | --- |
| `xlcells` | Lesen und Schreiben, die Fassaden | 1,4 MB | keines | ab 3.9 |
| `xlcells-engine` | Rechenkern, DataFrames, Regionen, Speicher, Aussehen | 4,0 MB | das der Umgebung (`polars>=2,<3`, wird mitinstalliert) | ab 3.11 |
| `xlcells-engine-bundled` | dasselbe mit eingebautem Polars 0.55.2 | 21,5 MB | eigenes, zusätzlich | ab 3.9 |

Ohne Rechenkern arbeitet alles außer `wb.calculate()`, `ws.to_polars()`, `wb.regions`, `wb.to_store()`, `cell.look` und
`xlcells.polars.read_excel`; diese nennen dann das fehlende Paket. Sind beide Rechenkerne installiert, nimmt `xlcells`
`xlcells-engine`; `XLCELLS_ENGINE=bundled` (oder `environment`) wählt ausdrücklich. Beide haben dieselbe Schnittstelle
(`engine_info()` sagt, welcher es ist und mit welchem Polars). pandas wird nirgends bedient: DataFrames sind Polars
(`to_polars`) oder Arrow (`to_arrow`). Release-Bauten sind ohne Symbole und über Paketgrenzen optimiert (`strip`,
Thin-LTO). Python-Umgebung mit uv: `uv sync` im Wurzelverzeichnis baut alle drei Pakete (editierbar, mit maturin) samt Testabhängigkeiten nach `.venv`; `uv run pytest bindings/python/tests` lässt die Tests laufen. Bauen: `tools/build_bindings.sh`. Für die Arbeit an der Python-Seite legt `tools/dev_python.sh` die Erweiterung neben die Quellen und lässt das installierte Paket auf sie zeigen. Die pyExcelerator-Ebene ist ein Nachbau der
Schnittstelle; es wird kein Code von pyExcelerator übernommen (dessen Lizenz hat eine
Werbeklausel). Seine Beispielskripte werden zur Testzeit geladen, nicht eingecheckt.

### Python-Fassaden

- `xlcells.openpyxl`: `Workbook`, `load_workbook` (auch `data_only`, `read_only`; liest auch `.xls`, das openpyxl
  ablehnt), `Workbook(write_only=True)`, `wb.calculate()` (Formeln berechnen, siehe Polars-Adapter), `ws["A1"]`, `ws.cell`, `ws.append`, `iter_rows`,
  `merge_cells`, `number_format`, `row_dimensions[...].height`,
  `column_dimensions[...].width`, `freeze_panes`, `ArrayFormula`, definierte Namen (`wb.defined_names`, `ws.defined_names`),
  Hyperlinks (`cell.hyperlink`), Autofilter (`ws.auto_filter.ref`), Datenprüfung (`DataValidation`),
  bedingte Formate als Objekte wie bei openpyxl (`CellIsRule`, `FormulaRule`, `ColorScaleRule`, `DataBarRule`,
  `IconSetRule`, `Rule` mit allen Regeltypen, `FormatObject`, `DataBar`, `IconSet`, `ColorScale`; geladene Regeln
  lassen sich ändern), Rich Text (`CellRichText`, `TextBlock`, `InlineFont`, `load_workbook(rich_text=True)`;
  formatierte Läufe bleiben auch ohne `rich_text` beim Speichern erhalten, openpyxl verliert sie dort),
  Autofilter mit Kriterien und Sortierung (`add_filter_column`, `add_sort_condition`, `FilterColumn`, `Filters`,
  `CustomFilters`, `Top10`, `DynamicFilter`, `ColorFilter`, `IconFilter`, `SortState`), `ws["A"]`, `ws["2:4"]`, `ws[3]`,
  Zeilen und Spalten mit allem, was sie haben (`row_dimensions`, `column_dimensions`: Höhe, Breite, ausgeblendet,
  Gliederung mit `group`, eingeklappt, dicke Ränder, Format ganzer Zeilen und Spalten), Tabellen (`Table`,
  `TableColumn`, `TableStyleInfo`, `ws.add_table`, `ws.tables`, Ergebniszeilen und berechnete Spalten),
  Mappenschutz (`wb.security`, `WorkbookProtection`), Dokumenteigenschaften (`wb.properties`, `wb.custom_doc_props`
  mit allen Werttypen), Berechnung (`wb.calculation`), Fenster und aktives Blatt (`wb.views`), `wb.code_name`, Kommentare
  (`cell.comment`), Bilder (`ws.add_image`, PNG/JPEG/GIF/BMP ohne Zusatzbibliothek), `wb.save`.
  Diagramme: `BarChart`, `LineChart`, `PieChart`, `ScatterChart` mit `Reference` und `Series` (`xlcells.openpyxl.chart`);
  Diagramme aus einer Datei laufen als Ganzes mit durch; sie lassen sich verschieben (`chart.anchor`) und entfernen
  (`ws._charts.remove`), ihr Inhalt (Reihen, Titel, Achsen) lässt sich nicht ändern.
  Druck und Ansicht: `ws.page_setup`, `ws.page_margins`, `ws.print_options`, Kopf- und Fußzeilen (`ws.oddHeader.center.text`,
  `ws.HeaderFooter`), `ws.print_area`, `ws.print_title_rows`/`print_title_cols`, Seitenumbrüche (`ws.row_breaks`),
  `ws.sheet_properties` (Registerfarbe, Gliederung, an Seite anpassen), `ws.sheet_view` (Zoom, Gitternetz, Ansicht,
  Auswahl), `ws.sheet_format`, Blattschutz `ws.protection` (mit Excels Passwort-Hash). Blattstruktur: `insert_rows`,
  `delete_rows`, `insert_cols`, `delete_cols`, `move_range` (auch `translate=True`), `wb.copy_worksheet`, `wb.move_sheet`.
  Hilfsmodule an openpyxls Stellen: `utils.cell`, `utils.datetime`, `formula.Tokenizer`, `formula.translate.Translator`,
  `worksheet.cell_range` (`CellRange`, `MultiCellRange`), `styles.numbers`, `styles.colors`, `styles.fills`,
  `styles.borders`, `cell.cell`.
  Formatierung im vollen Umfang von openpyxl: `Font` (alle Attribute, Themen- und indizierte Farben mit Tönung,
  Hoch-/Tiefstellung, Schema), `PatternFill` (alle Muster, Vorder- und Hintergrundfarbe), `GradientFill` mit `Stop`,
  `Border` mit allen Seiten (auch Diagonale, innere Linien), `Alignment` (alle Attribute), `Protection`,
  benannte Stile (`NamedStyle`, `wb.add_named_style`, `cell.style = "Good"` mit den 50 eingebauten Stilen),
  `quotePrefix`, `pivotButton`. Was eine Datei an Formatierung mehr enthält als die Felder des Kerns, läuft als XML
  des jeweiligen Teils mit (Markup-Kompatibilität: `mc:Choice` wird übergangen, `mc:Fallback` gelesen).
  Eine geladene Mappe wird **auf die Originaldatei** gespeichert: Diagramme, Bilder, Kommentare, definierte Namen,
  bedingte Formate, Datenprüfung, Pivot-Tabellen, Makros und Dokumenteigenschaften bleiben bytegenau erhalten;
  unveränderte Blätter ebenfalls. In einem geänderten Blatt werden die Zelldaten neu geschrieben.
- `xlcells.polars`: `read_cells` (lange Tabelle, die Basis für alles Weitere), `read_excel`, `write_excel`.

Die Tabelle der Sprachanbindungen hält Text zusammenhängend (bis 4 GiB). Eine geteilte Zeichenkette, die in vielen
Zellen steht, kann weit mehr Text ergeben, als die Datei enthält (POIs `poc-shared-strings.xlsx`: 12.000 Zellen × 1 MB);
solche Mappen werden vor dem Anlegen des Speichers mit einem Fehler abgelehnt, im Rust-Kern teilen sich die Zellen
die Zeichenkette. Polars 2.1 liest Spalten namens `*` oder `^…$` nicht (auch nicht aus eigener Datei); der Vergleichstest
nimmt sie aus.

Vergleich mit openpyxl über den Korpus (`tools/check_openpyxl_facade.py corpus`): 1.907 Dateien, je mit und
ohne `data_only`, 6.440 Blätter, 2,73 Mio. Zellen. Werte, Typen, Datumsumwandlung, Zahlenformate, fett/kursiv,
Verbünde und Array-Formeln stimmen überein. Verbleibende Unterschiede beim Laden:

- Steuerzeichen-Schreibweise `_x000D_`: wir dekodieren sie, openpyxl nicht.
- Leeres Formelelement: openpyxl liefert `"="`, wir den gespeicherten Wert.
- Datentabellen-Formeln sind bei openpyxl Objekte, bei uns Text.
- Blattgröße: leere Zellen ohne Stil zählen bei openpyxl mit.
- Makroblätter lesen wir nicht.

Abdeckung der öffentlichen openpyxl-Schnittstelle (`tools/openpyxl_coverage.py`: Namen der Module und Klassen, die
Skripte benutzen, unter demselben Pfad): 455 von 558 Namen (81,5 %; vorher 76,2 %, 70,8 %, 64,3 % und 23,8 %).

Zeilen und Spalten, Tabellen, Mappeneinstellungen über den Korpus: alle 1.433 Dateien zeigen in openpyxl und der Fassade
dieselben Zeilen- und Spaltenattribute, Tabellen (84 Dateien, 117 Tabellen; auch nach dem Speichern) und
Einstellungen. LibreOffice liest aus den geschriebenen Dateien Tabellen, ausgeblendete und gegliederte Zeilen und
Spalten, Höhen und Breiten. Bewusste Abweichungen von openpyxl:
- Eine Spalte, der nur Ausblenden oder Gliederung gegeben wird, behält ihre Breite; openpyxl schreibt dann Breite 13.
- Gegliederte Zeilen und Spalten tragen ihre höchste Ebene in `sheetFormatPr` ein (Excel braucht sie für die
  Gliederungsknöpfe); openpyxl lässt sie weg.
- Ob eine Zeile oder Spalte ein Format hat, entscheidet der Inhalt des Formats; openpyxl zählt die Nummern der
  Formatteile in der Datei (eine Kopie des Standardformats unter anderer Nummer gilt dort als Format). Zeilen und
  Spalten ohne Format zeigen das Normal-Format (wie Excel), openpyxl die erste Schrift und Füllung der Datei.
- Dokumenteigenschaften werden über die Paketbeziehung gesucht (wie es OPC vorsieht), dann am üblichen Ort;
  openpyxl sieht nur `docProps/core.xml` an.
- Speichern setzt wie openpyxl und Excel den Änderungszeitpunkt; sonst bleibt `core.xml` bytegleich. Eine Datei
  ohne Dokumenteigenschaften bekommt nur welche, wenn welche gesetzt werden.

Rich Text, bedingte Formate und Autofilter über den Korpus: 39 Dateien mit formatiertem Text (24.065 Zellen), 42 mit
bedingten Formaten, 17 mit Autofilter zeigen in openpyxl und der Fassade dieselben Objekte. openpyxl verwirft eine Regel,
deren Schwelle einen definierten Namen nennt (`simple-monthly-budget.xlsx`); die Fassade behält sie.
Im Kern laufen formatierte Läufe (`<r>`, Lautschrift) als XML des String-Eintrags mit (`Cells::rich`), geteilt wie der
Text; `rich_text` gibt den reinen Text eines solchen Eintrags.

Formatierung über den Korpus (`pytest bindings/python/tests/test_openpyxl_styles_full.py`): 1.433 Dateien, je bis zu
2.000 Zellen pro Blatt, zeigen in openpyxl und der Fassade dieselbe Formatierung (Zahlenformat, Schrift samt Farbtyp
und Tönung, Füllung, alle Rahmenseiten, Ausrichtung, Schutz, Stilname). Ausnahmen: verbundene Zellen (openpyxl gibt
ihnen ein neues Format) und Mappen ohne Schriftliste, für die openpyxl seine eigene Normal-Schrift (Calibri 12) statt
Excels (Calibri 11) annimmt. Ohne Angabe schreibt der Kern Excels Standardschrift (Calibri 11, Themenfarbe 1,
Schema minor) wie Excel und openpyxl.

Wo die Fassade bewusst von openpyxl abweicht, weil openpyxl Fehler macht: Der Formel-Übersetzer behält Leerraum,
Zeilenumbrüche und den Blatt- oder Mappennamen vor definierten Namen (openpyxl verliert sie); ein Bereich, dessen Ende
das Blatt verlässt, wird ganz zu `#REF!`; `coordinate_to_tuple` liest auch `$A$1`; Kopfzeilentext ohne Abschnittscode
bleibt erhalten (openpyxl verwirft ihn).

Noch nicht über die Fassade steuerbar (bleibt beim Laden und Speichern aber erhalten): den Inhalt geladener Diagramme ändern, weitere Diagrammtypen (Fläche, Blase, Netz,
Kurs, 3D), Datenbeschriftungen, Trendlinien, Sekundärachsen, Formen und Gruppen in Zeichnungen, Formatierung innerhalb
von Kommentaren.
Beim Speichern auf das Original können Blätter hinzugefügt, entfernt, umbenannt und umsortiert werden; in einem
geänderten Blatt werden nur die geänderten Zellen ersetzt, alle anderen Zellen und Zeilen behalten ihr XML
(formatierter Text, Zeilenattribute). Nur wenn die Zeilenhöhen geändert wurden, ein verteilter Formelblock
angeschnitten würde oder das Blatt keine expliziten Zellbezüge trägt, werden die Zelldaten des Blatts neu
geschrieben. Die Kinder von `<worksheet>`, die das Modell nicht als Felder kennt (Druck, Ansicht, Schutz, Filter mit
Kriterien, …), laufen als XML-Stücke durch (`Sheet::extra`) und werden einzeln ersetzt, wenn sie sich ändern.
Ebenso die Kinder von `<workbook>` (`Workbook::extra`: Schutz, Fenster, Berechnung, …) und die Dokumenteigenschaften
(`Workbook::properties`). Zeilen- und Spaltenattribute sind im Modell eigene Felder (`Sheet::rows`, `Sheet::columns`:
Höhe, Breite, ausgeblendet, Gliederung, eingeklappt, Format, dicke Ränder). Tabellen laufen als XML ihres Teils
(`Sheet::tables`); ihre Nummern bleiben, wenn sie eindeutig sind, sonst werden alle neu vergeben, Anzeigenamen
müssen in der Mappe eindeutig sein. Ändern sich Tabellen beim Speichern auf das Original, wird die Datei neu
geschrieben.

### Tests

| Prüfung | Befehl | Gegenprobe |
| --- | --- | --- |
| Leser gegen calamine, Rundlauf lesen → schreiben → lesen | `cargo test --release -p xl-golden --test corpus_cells` | calamine, 3.442 Dateien |
| Beispielskripte von pyExcelerator, unverändert | `pytest bindings/python/tests/test_pyexcelerator_examples.py` | xlwt + xlrd, openpyxl, LibreOffice |
| Zufallsmappen | `pytest bindings/python/tests/test_property.py` | openpyxl, XlsxWriter, python-calamine |
| Schema ISO/IEC 29500 | `pytest bindings/python/tests/test_schema.py` | offizielle XSD |
| Gleiche Bytes in allen Sprachen | `tests/conformance/` (Rust erzeugt die Erwartung) | Rust ↔ Python ↔ JS ↔ Dart |
| openpyxl-Fassade | `pytest bindings/python/tests/test_openpyxl_facade.py`, `tools/check_openpyxl_facade.py corpus` | openpyxl: gleiche Skripte, gleiche Dateien |
| Druck, Ansicht, Schutz, Hilfsfunktionen | `pytest bindings/python/tests/test_openpyxl_sheet_settings.py` | openpyxl (gleiche Skripte; Passwort-Hash, Kopfzeilen, Koordinaten, Datumswerte mit Hypothesis) |
| Formel-Zerleger und -Übersetzer | `pytest bindings/python/tests/test_openpyxl_formula.py` | openpyxl über alle Korpusformeln |
| Zeilen, Spalten, Bereiche verschieben | `pytest bindings/python/tests/test_openpyxl_structure.py` | openpyxl (zufällige Folgen mit Hypothesis) |
| Formatierung und Erhalt beim Speichern | `pytest bindings/python/tests/test_openpyxl_styles_and_keep.py`, `cargo test --release -p xl-golden --test corpus_cells saving_onto` | openpyxl; Teilevergleich der Pakete |
| Rich Text | `pytest bindings/python/tests/test_openpyxl_rich_text.py` | openpyxl: gleiche Skripte, Zufallstexte (Hypothesis), Korpus |
| Datenbalken, Symbolsätze, alle Regeltypen | `pytest bindings/python/tests/test_openpyxl_conditional_full.py` | openpyxl: gleiche Skripte, Korpus |
| Zeilen und Spalten: Höhe, Breite, ausgeblendet, Gliederung | `pytest bindings/python/tests/test_openpyxl_dimensions.py` | openpyxl: gleiche Skripte, 1.433 Korpusdateien |
| Tabellen | `pytest bindings/python/tests/test_openpyxl_tables.py` | openpyxl (auch nach dem Speichern), LibreOffice, Schema |
| Mappenschutz, Dokumenteigenschaften, Berechnung, Fenster | `pytest bindings/python/tests/test_openpyxl_workbook_settings.py` | openpyxl: gleiche Skripte, 1.433 Korpusdateien; Regeln für `core.xml` aus ECMA-376 Teil 2 |
| Autofilter mit Kriterien, Sortierung | `pytest bindings/python/tests/test_openpyxl_autofilter.py` | openpyxl, Schema ISO/IEC 29500 |
| Volles Stilmodell, benannte Stile | `pytest bindings/python/tests/test_openpyxl_styles_full.py` | openpyxl: gleiche Skripte, 1.433 Korpusdateien Zelle für Zelle |
| Namen, Hyperlinks, Autofilter, Datenprüfung | `pytest bindings/python/tests/test_openpyxl_objects.py` | openpyxl |
| Bedingte Formate, Kommentare | `pytest bindings/python/tests/test_openpyxl_conditional.py bindings/python/tests/test_openpyxl_comments.py` | openpyxl |
| Bilder, Erhalt von Diagrammen | `pytest bindings/python/tests/test_openpyxl_images.py` | openpyxl |
| Diagramme | `pytest bindings/python/tests/test_openpyxl_charts.py` | openpyxl, Diagramm-XSD |
| Polars-Fassade | `pytest bindings/python/tests/test_polars_facade.py` | Polars mit calamine und openpyxl |
| ISO-Datumszellen, `read_only`, `write_only` | `pytest bindings/python/tests/test_openpyxl_modes.py` | openpyxl: gleiche Skripte, Korpusdateien |
| `.xls` lesen | `XLS_STEP=1 cargo test --release -p xl-xls --test golden` | calamine (21.342 Dateien), LibreOffice-Konvertierungen (1.485), xlrd |
| Berechnen | `cargo test --release -p xl-polars --test calculate`, `pytest bindings/python/tests/test_openpyxl_calculate_and_xls.py` | LibreOffice: gespeicherte und neu berechnete Werte |
| Fremde Leser auf neu geschriebenen Korpusdateien | `xlg rewrite corpus out src-` und `tools/check_rewritten.py out/rewritten` | openpyxl, python-calamine |

Sprachtests: `cd bindings/js && node --test`, `cd bindings/dart && XLCELLS_LIB=../../target/release/libxlcells.so dart test`,
`cc -Ibindings/c/include bindings/c/tests/smoke.c -Ltarget/release -lxlcells -lm`.

### Altes Format `.xls` (`crates/xl-xls`)

Liest Excel 97–2003 (BIFF8) in dasselbe Modell wie `.xlsx`: Compound File mit Prüfung jeder Sektorkette (kein
Endlos-Lauf, keine Speicheranforderung über die Dateigröße), dann die Datensätze: Zellen (`LABELSST`, `RK`, `MULRK`,
`NUMBER`, `BOOLERR`, `FORMULA` mit `STRING`), geteilte und Array-Formeln, Zahlenformate, Schriften, Füllungen,
Rahmen, Ausrichtung, Schutz, Palette, Spalten und Zeilen, Verbünde, fixierte Bereiche, Sichtbarkeit der Blätter,
1904-Datumssystem, definierte Namen (auch eingebaute, blattlokale), externe Arbeitsmappen (`[1]Blatt!A1`, mit den
zwischengespeicherten Zellen und den Definitionen ihrer Namen, `EXTERNNAME` mit `itabFirst`/`itabLast`), Add-in-Funktionen
und die Berechnungseinstellungen (`CALCMODE`, `CALCCOUNT`, `ITERATION`, `DELTA`, `PRECISION` als `calcPr`). Formeln werden aus dem Token-Strom in die Schreibweise der
`.xlsx` zurückübersetzt (Tabelle der Funktionsnummern aus calamine, MIT). `load_workbook` erkennt `.xls` am Inhalt;
gespeichert wird als neue `.xlsx`.

Gegenproben über den ganzen Korpus: 21.342 `.xls`-Dateien gegen calamine — 142,6 Mio. Zellen, kein abweichender Wert
(außer calamines eigenen Fehlern bei 16-Bit-Texten und Texten über `CONTINUE`, dort stimmen wir mit xlrd überein).
1.485 Enron-Dateien gegen ihre LibreOffice-Konvertierung: 2,1 Mio. Formeln, 8 abweichend; was LibreOffice anders
schreibt (Zahlen mit 15 Stellen, `TRUE()`, ganze Zeilen als `$A$2:$IV9`, Bezüge auf gelöschte Zellen als die alte
Zelle, Namen fremder Mappen falsch zugeordnet) ist im Test einzeln benannt. Wir folgen Excel: `Blatt!#REF!`,
`'1Q'!A1` und `'NP15'!A1` in Anführungszeichen, ganze Zeilen und Spalten bleiben ganze Zeilen und Spalten.

Nicht gelesen: Excel 5.0/95 (BIFF5, 3 % der Enron-Dateien; Fehlermeldung), verschlüsselte Dateien, Diagramm- und
Makroblätter, Diagramme, Bilder, Kommentare, Hyperlinks, bedingte Formate, Datenprüfung, formatierte Textläufe,
Datentabellen-Formeln (`TABLE`, nur ihre Werte). Schreiben von `.xls` ist nicht vorgesehen.

### Noch nicht enthalten

Die pyExcelerator-Ebene nimmt Schriftart, Farben, Rahmen, Ausrichtung, Zeilenhöhen, Hyperlinks, Bilder, Fenster- und
Druckeinstellungen an, schreibt sie aber nicht (die openpyxl-Fassade kann alles davon). Ungeklärt bis zur Prüfung mit MS Excel: ob Text ohne
`xml:space="preserve"` beim Lesen beschnitten wird (calamine tut es; LibreOffice, openpyxl
und wir nicht).

## Rechenkern (`crates/xl-engine`) und seine Ausführung auf Polars (`crates/xl-polars`)

Der Rechenkern hängt an keiner DataFrame-Bibliothek. `xl-engine` übersetzt Formeln in Spaltenausdrücke einer eigenen
Zwischenform (`ir`: der Teil von Polars' Wortschatz, den der Rechenkern braucht, rund 60 Operationen, mit Polars' Namen
und Bedeutung) und beschreibt jede Auswertung als Plan: eine Eingabetabelle, Zwischenspalten in Schichten, Ausgaben.
Ein `ir::Executor` führt Pläne aus:

| Ausführung | Ort | benutzt von |
| --- | --- | --- |
| Polars in Rust (0.55.2), eingebaut | `xl-polars` (`builtin::PolarsExecutor`) | `xl-mcp`, Rust-Tests, `xlcells-engine-bundled` |
| Polars der Python-Umgebung | `bindings/python-engine`: der Plan geht als JSON (`ir::wire`), die Tabellen als Arrow; `xlcells_engine/_shim.py` (rund 150 Zeilen) baut daraus `pl.Expr` | `xlcells-engine` |
| Polars für Dart, JavaScript | später; dasselbe Plan-Format | |

Daneben in `xl-engine`: typisierte Tabellen (`data::Frame`), Regionen, Zurückschreiben, das Polars-Programm und der
Speicher (Parquet über eine `store::Backend`-Schnittstelle: Polars in Rust oder das der Umgebung). `xl-arrow` gibt
Tabellen als Arrow-C-Stream heraus und nimmt sie an (arrow-rs, ohne Polars). `xl-polars` ist die dünne Schicht darüber:
Ausführung, Umwandlung in Polars-DataFrames, Parquet, und die alten Einstiege (`lift::calculate`, `store::run`).

Gemessen (`cargo run --release -p xl-polars --example baseline` schreibt jeden berechneten Wert von 1.278 Mappen,
`tools/engine_drift.py` rechnet dieselben Mappen mit `xlcells-engine` und vergleicht Zelle für Zelle):

| | |
| --- | --- |
| Umstellung auf die Zwischenform und Aufteilung der Bibliotheken | 548.684 Zeilen vorher und nachher identisch |
| Polars der Umgebung (2.1.0) gegen eingebautes (0.55.2): Formelzellen | 545.050 in 1.178 Mappen |
| bitgleich | 539.561 (98,99 %) |
| Zahlen, die sich in den letzten Bits unterscheiden | 5.489; größter relativer Unterschied 3,2e-14 |
| nur von einem der beiden berechnet, oder anderer Art | 0 |
| Rechenzeit | gut doppelt so lang wie eingebaut (ein Übergang nach Python je Formelgruppe und Runde) |

Die Unterschiede kommen vom Teilen: Polars 2.1 teilt, 0.55.2 multipliziert eine Spalte mit dem Kehrwert einer Konstanten.
Von den 397 Zellen in xlsx-Dateien, die sich jenseits der 15. Stelle unterscheiden, liegt das Polars der Umgebung bei
342 näher am gespeicherten Wert, das eingebaute bei 52 (`TRUNC(-A2/3,1)` ergibt -0,3 statt -0,30000000000000004).

### Pläne, die für andere Daten gelten (`lift::Options::param`)

Der Rechenkern setzt von sich aus feste Zellen (absolute Zeile, benannte Einzelzellen), Bereiche und Nachschlagetabellen mit
ihren Werten in die Ausdrücke ein und übersetzt bei jedem Rechnen neu. Für ein wiederverwendbares Programm gibt es den
Modus `param`: feste Zellen und die Zahlen fester Bereiche (Summe, Anzahl, Kleinstes, Größtes, Quadratsumme) sind Eingaben
des Plans. Jede andere Lesung der Mappe beim Übersetzen läuft über einen einzigen Zugriff und steht in
`LiftedRun::assumptions`: Zellen, deren Wert den Ausdruck geformt hat (Nachschlagetabellen, Entscheidungen), die Ausdehnung
von Blättern, die Art (Text, Fehler) der gelesenen Spalten und festen Zellen. Der Plan gilt, solange das gilt.

`lift::reuse_check` übersetzt eine Mappe einmal, lässt den Plan auf geänderten Daten laufen und vergleicht mit der geänderten
Mappe (`cargo run --release -p xl-polars --example reuse DIR STEP`). Gemessen an 1.044 Mappen (241.747 übersetzte Zellen), alle
Zahlen, die keine Formeln sind, mit 1,37 multipliziert:

| Übersetzung | auf den geänderten Daten richtig | Annahme verletzt (Abweichung erwartet) | falsch trotz gültiger Annahmen |
| --- | --- | --- | --- |
| feste Zellen als Konstanten | 67,7 % | 25,8 % | 0 |
| feste Zellen als Eingaben | 80,6 % | 12,8 % | 0 |
| dazu feste Bereiche als Eingaben | 84,1 % | 9,3 % | 0 |

Im Param-Modus rechnet der Rechenkern dieselben Werte wie vorher: gegen den Bezugslauf der 1.278 Mappen sind 539.554 Zellen
bitgleich, 5.496 unterscheiden sich in den letzten Bits (höchstens 3,2e-14, es wird durch eine Spalte statt durch eine
Konstante geteilt), keine wird nur in einem Modus berechnet; die golden-Tests gegen Excel und LibreOffice bestehen in beiden
Modi (`XL_PARAM=1`).

Der Rest der Annahmen betrifft Nachschlagetabellen, Blöcke von Werten und Entscheidungen, die vom Wert einer Zelle abhängen
(etwa 9 %): Dafür bleibt der Wert eingesetzt, und das Programm nimmt dort die Werte des Rechenkerns.

### Das Programm schreibt den Plan aus (`pycode.rs`, `codegen.rs`)

`to_polars_code` hat drei Arten, eine berechnete Spalte oder Zelle zu schreiben (`code.items`, Spalte `how`): `polars`
(Polars' eigene Operationen, lesbar), `plan` (der Plan des Rechenkerns als Polars-Code, mit der Genauigkeit von Excel) und
`engine` (die Werte von `wb.evaluate`). Der Plan liest die Daten des Programms: Spalten aus den Frames der Regionen, feste
Zellen aus benannten Einzelzellen, Tabellenzellen oder der Mappe, Bereiche als Tabellenspalten oder Zellen. Was er für seine
Eingaben vorausgesetzt hat (Zahl, Text), prüft das Programm vor dem Rechnen; sonst nimmt es die Werte des Rechenkerns.
Gemessen (`tools/polars_code_check.py DIR STEP MAX exact`, 103 Mappen der LibreOffice-Enron-Daten): 17.422 von 20.537 Posten
(84,8 %) stehen als Plan im Programm, alle 17.422 geben den Wert der Mappe. Häufigste Gründe für die Werte des Rechenkerns:
Formel hängt von der Länge des Blatts ab (1.527), Verweis auf eine andere Mappe (523), flüchtige Funktion (286), Bereich über
mehrere Blätter (264), Bereich über einen Teil einer Tabelle (92).

- `cells_frame(&Workbook)`: die Zelltabelle als Polars-DataFrame (eine Zeile pro Zelle). Nur hier tritt die Mappe in Polars ein.
- `frame::frame` / `frame::sheet_frame`: ein Bereich eines Blatts als typisierte Tabelle (ein Datentyp je Spalte: Int64,
  Float64, Boolean, Date, Datetime, Time, Duration, String; gemischte Spalten String, leere Zellen und Fehler null).
  `xl_arrow::frame_to_stream` gibt sie als Arrow-C-Stream heraus: So verlassen Tabellen Rust, unabhängig von der Polars-Version
  des Empfängers. In Python: `ws.to_polars()`, `ws.to_polars("A1:F40")`, `ws.to_arrow()` an der
  openpyxl-Fassade; `xlcells.polars.read_excel` geht denselben Weg (vorher: Typbestimmung in Python über Listen).
- `lift::runs`: Formeln, die in einer Spalte heruntergezogen wurden (gleiche Formel in relativer Lesart, mindestens 3 Zellen),
  werden als *eine* Spaltenformel erkannt. `lift::runs_from(wb, 1)` nimmt jede Formelzelle (Einzelzellen als Lauf der Länge 1).
- `lift::lift` / `lift::lift_from`: Spaltenformeln werden in Polars-Ausdrücke übersetzt, über die Zeilen ausgewertet und gegen
  die in der Datei gespeicherten Werte geprüft. Nur was jeden Wert trifft, gilt als übersetzt.
- `lift::calculate`: berechnet eine Mappe mit diesen Ausdrücken neu — Runde um Runde, bis sich kein Wert mehr ändert.
  Nicht berechnet (und mit Grund gezählt) werden Zellen ohne Übersetzung, Zellen auf einem Zirkelbezug und alle Zellen,
  die eine solche lesen (Abhängigkeitsgraph in `lift/deps.rs`, auch über Namen); wer `INDIRECT`, `OFFSET` und ähnliche
  benutzt, gilt als unsicher, sobald irgendeine Zelle nicht berechnet ist. Fehlerergebnisse sehen die Leser als Fehler;
  nur wer nach der Art des Fehlers fragt (`ISNA`, `IFNA`, `ERROR.TYPE`, `ISERR`), wird nicht berechnet. In Python:
  `wb.calculate()`; bleibt etwas unberechnet, setzt die Mappe `fullCalcOnLoad`, damit Excel den Rest rechnet.
  Gegenprobe: alle 1.485 Enron-Dateien mit von LibreOffice berechneten Werten, Formelwerte gelöscht und neu berechnet
  (`CALC_STEP=1`, 30 Minuten; ohne die Variable jede 15. Datei). 933.065 Formelzellen, rund 748.000 berechnet (80 %;
  in der Stichprobe 99 %; der Rest vor allem Fehlerergebnisse und deren Leser). Kein abweichender Wert außer
  benannten LibreOffice-Abweichungen, die der Test einzeln zählt: `DATE` mit Jahr unter 1900 (Excel zählt 1900 dazu),
  Namen fremder Mappen (`[1]!Name`, LibreOffice kann sie nicht lesen) samt der Zellen dahinter, Vergleichsergebnisse
  als 1/0, Restwerte nahe 0 nach Summen großer Zahlen (LibreOffice summiert kompensiert und schreibt Werte mit
  15 Stellen). Der Gesamtlauf fand und behob: Vergleiche mit 15-Stellen-Toleranz (`0.1+0.2=0.3`), Zahlen im Text mit
  15 Stellen (`"NPV "&9.000000000000002` ergibt `NPV 9`), `MAX`/`MIN` über lauter Fehler, ein Fehler als Suchschlüssel,
  `MATCH(x;r;)` sucht exakt, `TEXT(0;"###-##")`, fehlende Zellen anderer Mappen galten als leer, `#REF!` als Argument,
  Namen in Zellschreibweise (`NXB3`, in .xls gültig), nicht eingeschwungene und nicht erfasste Zellen.
  Fehlersuche: `XL_TRACE="Blatt,Zeile,Spalte;…"` (0-basiert) zeigt die Werte einer Zelle Runde für Runde und den Grund,
  `XL_CYCLES=1` die Zellen auf Zirkelbezügen und warum sie (nicht) berechnet wurden.
- Zirkelbezüge: ohne iterative Berechnung bleiben sie (wie in Excel) unberechnet. Mit `iterate` in `calcPr` werden sie
  wiederholt durchgerechnet, mit `iterateCount` und `iterateDelta` der Mappe; wir rechnen weiter bis auf ein Tausendstel
  des Deltas, damit das Ergebnis nicht vom Startwert abhängt. Was nicht einschwingt, bleibt unberechnet. Mit „Genauigkeit
  wie angezeigt“ (`fullPrecision="0"`) wird nichts berechnet. `.xls` liefert diese Einstellungen jetzt mit.
- Gegenprobe gegen Excel selbst: die von Excel gespeicherten Werte der Enron-`.xls`-Dateien
  (`cargo test --release -p xl-polars --test calculate_xls`; jede 40. Datei, `CALC_STEP=1` alle 20.000). Stichprobe:
  402 Dateien, 201.004 Formelzellen, 177.422 berechnet, kein abweichender Wert. Ausgenommen und gezählt: Dateien mit
  manueller Berechnung (Werte evtl. veraltet), Funktionen des Analyse-Add-ins, die Excel vor 2007 ohne geladenes Add-in als
  `#NAME?` speicherte, `IRR`/`XIRR`/`RATE` innerhalb von Excels Lösungsgenauigkeit (1e-7), Formeln über andere Mappen, deren
  gespeicherter Wert nicht zum mitgespeicherten Stand der anderen Mappe passt.
- Zirkelbezüge gegen Excel: von 671 Dateien mit iterativer, automatischer Berechnung haben 12 echte Zirkelbezüge. Alle
  Formelzellen starten 1 % neben Excels Wert; in 9 Dateien (bis 120.000 Zellen, `CALC_MAX_CELLS=120000`; Standard 60.000)
  werden 570 Zellen auf Zirkeln durch Iteration berechnet und treffen Excel. Bei einem langsam einschwingenden Modell
  (1.984 Zellen) hält die Iteration woanders an als Excel; Excels Werte sind dort auch für uns in Ruhe (von ihnen aus
  bewegt sich nichts), also stimmen die Formeln. Die übrigen Zirkel lesen Zellen, die wir nicht berechnen (Namen fremder
  Mappen, `SPLINE`, ...), und bleiben unberechnet.
- Gefunden und behoben durch die Excel-Gegenprobe: ein Fehlerergebnis, das eine nicht berechnete Zelle liest, galt als
  echter Fehler und wurde weitergereicht (`DSUM` über eine Spalte mit `NOW()`).

### Wertmodell und Auswertung

Eine Zelle ist Zahl, Text, leer oder Fehlerwert. Jede gelesene Spalte tritt als Zahlenspalte ein (leer und Text sind null,
ein Fehlerwert ist NaN), wenn die gelesenen Zeilen Text enthalten zusätzlich als Textspalte und als Spalte „Text als Zahl“
(was Excel in Rechnungen aus `"1/31/2001"`, `"5%"` usw. liest). Ein Ergebnis kann je Zeile Zahl oder Text sein; ein Fehler ist
null und pflanzt sich wie in Excel fort. Mehrfach benutzte Teilausdrücke werden benannte Zwischenspalten
(`LiftedRun::lets`); lange Ketten werden in Teilen ausgewertet.

- **Wächter** (`LiftedRun::guards`): Bedingungen je Zeile, unter denen die Übersetzung nicht gilt (Platzhalter `* ? ~` im
  Suchschlüssel, Kriterien mit Operator als Text). Trifft eine Zeile zu, bleibt die Spaltenformel dem Zellrechner.
- **Fortschreibung über die eigene Spalte** (`LiftedRun::own`): Liest eine Spaltenformel frühere (oder nur spätere) Zeilen
  der eigenen Spalte, wird sie wiederholt ausgewertet, die eigene Spalte jeweils durch das letzte Ergebnis ersetzt, bis sich
  nichts mehr ändert. Die Rekursion hat zu gegebenem Start genau eine Lösung, der Fixpunkt ist sie; jede Runde legt
  mindestens eine weitere Zeile fest. `=C1+B2` und `=C1*B2` gehen schneller als `cum_sum`/`cum_prod`. Liest die Spalte
  ihre eigene Zelle oder Zeilen darüber *und* darunter innerhalb des Laufs, ist das ein Zirkelbezug und wird so benannt.
- **Feste Teile werden beim Übersetzen ausgerechnet** (mit denselben Regeln, auf einer Zeile): Argumente von `OFFSET`, die
  Art eines `MATCH`, `INDIRECT` mit festem Text, `CHOOSE` mit festem Index über Bezüge, Kriterien. Die Übersetzung hängt
  damit an den Daten in festen Zellen und Tabellen; ändern die sich, ist sie neu zu bilden (Vorgabe für die spätere
  Abhängigkeitsverfolgung).
- **Matrixformeln** (`{=SUM(A1:A5*B1:B5)}`) und `SUMPRODUCT` nehmen Bereiche an Stellen, die einen Wert erwarten, Zelle für
  Zelle; ungleich große Matrizen werden wie in Excel aufgefüllt (einzelne Zeile oder Spalte wiederholt, sonst `#N/A`).
  Ohne Matrixformel liefert ein Bereich dort die Zelle der eigenen Zeile oder Spalte (implizite Schnittmenge).
- **Matrixformeln über festen Bereichen** (`lift/array.rs`): Eine Matrixformel, deren Bereiche fest sind (in einer
  einzelnen Zelle immer), wird beim Übersetzen Element für Element nach Excels Regeln ausgerechnet: Vergleiche über
  Zahl, Text und Wahrheitswert, Text in Rechnungen, Fehler je Element, Verbreiterung einzelner Zeilen und Spalten, `IF`,
  `OFFSET`, `YEAR`/`MONTH`/`DAY`, `IS…`, `INDEX`, `LINEST`. Das Aggregat darum nimmt das Ergebnis als festen Bereich.
  Bis 4 Mio. Elemente; vorher wurde die Formel in einen Ausdruck je Element ausgerollt und scheiterte ab 400 Elementen.
- **Auswertung in Schichten**: Die Zwischenspalten werden nach Abhängigkeit in Schichten gelegt, jede Schicht ist ein
  Polars-Knoten; lange Summen werden als ausgeglichener Baum gebaut. Das hält auch Formeln mit Tausenden Teilausdrücken klein.

### Umfang

- Rechnen, Runden (`ROUND` sieht 15 Stellen), Vergleiche nach Excels Regeln, Logik, Prüfungen (`ISNA`/`ISERR` genau, wo die
  Fehlerart feststeht oder aus einem Nachschlagen kommt), Text (`&`, `CONCATENATE`, `LEFT`, `MID`, `FIND`, `SEARCH`, `TEXT`
  mit Datumsformaten, `DATEVALUE`, …).
- Datum mit Excels Kalender (Tag 0, 29.2.1900, 1904-System): `YEAR` … `DATE`, `EDATE`, `EOMONTH`, `WORKDAY` und
  `NETWORKDAYS` mit Feiertagen, `YEARFRAC` (Basis 0, 2, 3, 4).
- Aggregate über Zeilenzellen, feste, wandernde und wachsende Bereiche; `STDEV`/`VAR`; `COUNTA`, `MAXA`/`MINA`, `RANK`,
  `CORREL`; `SUMIF`/`COUNTIF`/`AVERAGEIF` und die `IFS`-Formen mit festen und zeilenweisen Kriterien (große feste Bereiche:
  Summen je Schlüssel einmal gebildet und nachgeschlagen; wachsende Bereiche: laufende Summen); `DSUM` und die übrigen
  D-Funktionen (Kriterienbereich fest).
- Nachschlagen: `VLOOKUP`, `HLOOKUP`, `MATCH`, `INDEX`, `LOOKUP` über eine gemeinsame Auswahl nach Position (`replace_strict`,
  `search_sorted`); genau, ungefähr, mit berechneter Spalte und zeilenweiser Art; ungefähr auch über Schlüssel mehrerer Arten
  (Schlüssel der gesuchten Art) und über ungeordnete Schlüssel (Excels Halbierungssuche, je Abschnitt zwischen den
  Schlüsseln ausgerechnet); Tabellen, die mit der Zeile wandern.
- Bezüge: eigene und andere Zeilen (`shift`), andere Blätter, feste Zellen und Bereiche, ganze Spalten und Zeilen, Ketten
  `A1:B1:C1`, Namen (auch blattlokal und Namen anderer Mappen aus deren Zwischenspeicher), `OFFSET` (vier Fälle, siehe
  Komponenten-Dokument), andere Arbeitsmappen über die Zwischenspeicher in `xl/externalLinks`.
- Finanzen: `PMT`, `PV`, `FV`, `IPMT`, `PPMT`, `NPV`, `IRR`, `SLN`, `XNPV`, `XIRR` (Newton ab 0,1), `VDB` und `DDB` (aus festen
  Argumenten beim Übersetzen gerechnet).
- Statistik und Weiteres: `NORMDIST`/`NORMSDIST` (Reihe und Kettenbruch, etwa 15 Stellen), `GROWTH`, `PERCENTILE`,
  `SUMSQ`, `AVERAGEA`, `SUBTOTAL` (ohne verschachtelte `SUBTOTAL`), `TEXT` mit Zahlenformaten (`0.00`, `#,##0`, `%`,
  `###-##-####`) und Uhrzeiten, `DOLLAR`, `TRIM`, `CHAR`, `ADDRESS`, `CELL` (Zeile, Spalte, Adresse, Inhalt), `TRANSPOSE`,
  `LINEST` mit Statistik (Householder-QR auf zentrierten Spalten; abhängige x-Spalten werden abgelehnt).

### Absicherung

- Goldtest: `tools/make_lift_golden.py` schreibt 428 Spaltenformeln über gemischte Spalten (auch Matrixformeln,
  Fortschreibungen, Datenbank- und Finanzfunktionen), lässt LibreOffice sie rechnen; der Test verlangt jede übersetzt und
  jeden Wert getroffen. Wo LibreOffice anders rechnet als Excel (Tage vor dem 1.3.1900, `FIND("")`, `TRUE` in Text,
  D-Funktionen mit Text ohne `=`, ungeordnete Schlüssel), stehen Fälle im handgeschriebenen Test oder sind nur am Korpus
  geprüft; mit Excel selbst ist nichts davon gelaufen.
- `tools/check_mismatch_xls.py` vergleicht jede abweichende Zelle zusätzlich mit dem Wert, den Excel in der ursprünglichen
  `.xls` gespeichert hat.
- `tools/lift_each.sh` misst Datei für Datei mit Speichergrenze; `tools/merge_lift.py` zählt zusammen.

### Messung

Korpus: Enron-Stichprobe (LibreOffice-Konvertierungen der `.xls`-Dateien), Datei für Datei mit `tools/lift_each.sh`
gemessen, zusammengezählt mit `tools/merge_lift.py`.

| Größe | Spaltenformeln (ab 3 Zellen) | jede Formelzelle (`LIFT_MIN_RUN=1`) |
| --- | --- | --- |
| Formelzellen | 2.133.499, davon 1.707.713 in 43.392 Spaltenformeln | 2.096.593 (1.483 Mappen; eine über der Speichergrenze) |
| übersetzt und geprüft | 1.642.315 (96,2 % der Spaltenformel-Zellen) | 2.019.406 (96,3 %) |
| übersetzt, Werte weichen ab | 8.487 | 8.985 |
| nicht übersetzt | 56.911 | 68.202 |

Eine Spaltenformel, die als Ganzes nicht geht, wird Zelle für Zelle versucht (höchstens eine Minute je Mappe): In einer
einzelnen Zelle sind alle Bezüge fest, damit sind `INDIRECT`-Texte, Kriterien, wandernde Tabellen und `OFFSET` bekannt.

**Was grundsätzlich nicht geht** (rund 68.200 Zellen, 3,25 % aller Formelzellen):

- Bezüge auf andere Arbeitsmappen, deren Werte die Datei nicht mitführt: 42.028 Zellen (Zellen, Namen, unvollständige
  Zwischenspeicher). Mit der anderen Mappe ginge es.
- Benutzerdefinierte Funktionen (VBA oder Add-ins, Code nicht in der Datei): 20.890 (`HPVAL`, `INDGENCOST`, `XSPRDOPT`,
  `BSD`, `EURO` mit acht Argumenten, `NEXTMONTH`, `AVERAGEPRICES`, `HPLNK`, `XCALCSKEW`, `HOLIDAY` und weitere).
- Flüchtige Funktionen `TODAY`, `NOW`, `RAND`: 3.974 (übersetzbar, nicht prüfbar).
- Datentabellen `TABLE()`: 925 (Neuberechnung der ganzen Mappe mit anderen Eingaben, kein Spaltenausdruck).
- DDE: 293. `CELL` über die zuletzt geänderte Zelle oder den Dateipfad: 74. Zirkelbezüge: 3.
- Namen und Formeln, die die Konvertierung zerstört hat (`#NAME!CANADA` statt eines blattlokalen Namens): 14.

**Offen**: eine Zelle (ein Bereich als Einzelwert, der in einer Spaltenformel teils neben den Zeilen liegt).

**Abweichungen** (8.984 Zellen): `tools/check_mismatch_xls.py` vergleicht Beispielzellen mit dem Wert in der ursprünglichen
`.xls`. Von 164 Beispielen treffen 54 Excels ursprünglichen Wert oder Fehler, der Wert in der konvertierten Datei stammt
von LibreOffice; die übrigen gehen auf die Konvertierung zurück (Eingangswerte mit weniger Stellen, ein falsch
zugeordneter blattlokaler Name, unvollständige Zwischenspeicher, Bezüge jenseits von Spalte `IV`, ein `FALSE`, das
LibreOffice als 0 speichert, `INDIRECT` auf Blätter namens `R5`, `R6`, die LibreOffice als Zellbezug liest). Dazu kommt
Auslöschung: eine Summe von Tausenden Werten um 10⁵, deren Ergebnis nahe 0 liegt (Excel 4,7·10⁻⁷, LibreOffice
−1,8·10⁻⁸, hier −1,1·10⁻⁷), hängt von der Reihenfolge der Additionen ab. Ungeklärt: eine Spaltenformel mit 1.724 Zellen.

`LIFT_SHOW=<Grund>` gibt die betroffenen Formeln aus, `LIFT_REASONS`, `LIFT_EXAMPLES`, `LIFT_SHARD=k/n` und
`LIFT_MIN_RUN` steuern die Messung.


## Von der Mappe zur Datenbankanwendung (`xl-engine`: `regions`, `apply`, `codegen`, `store`; `crates/xl-mcp`)

Ziel: Ein Agent (ChatGPT mit Python, Copilot über MCP) überführt eine bestehende Mappe selbständig in eine
Datenbankanwendung und kann den Weg zurück in die Mappe behalten. Alles Inhaltliche liegt in Rust; Python ist eine dünne
Schicht an der openpyxl-Fassade, der MCP-Server kommt ohne Python aus. Tabellen überqueren die Grenze als Arrow.

| Stufe | Rust | Python (openpyxl-Fassade) |
| --- | --- | --- |
| Tabellen heraus | `frame::frame`, `xl_arrow::frame_to_stream` | `ws.to_polars()`, `ws.to_arrow()` |
| Was die Mappe enthält | `regions::regions`: Excel-Tabellen; sonst Blöcke entlang leerer Zeilen/Spalten, daraus Tabellen (Kopfzeile aus Beschriftungen, einzelne Spalten dürfen unbenannt sein; Spalten je Werte oder eine heruntergezogene Formel, mit Ausnahmezeilen, siehe unten), Einzelzellen (benannt nach definiertem Namen, sonst Beschriftung links, sonst oben) und Blöcke (Zahlengitter). Logik je berechneter Spalte/Zelle als Formel und in strukturierter Schreibweise (`[@qty]*[@price]`, `SUM(Orders[amount])`, `Tax_rate`) | `wb.regions`, `ws.regions`, `region.columns`, `.cells`, `.to_polars()` |
| Zurück in die Zellen | `apply::write_region`: Werte der nicht berechneten Spalten; Tabellen wachsen und schrumpfen, berechnete Spalten werden mitgezogen, Formeln über ganze Tabellenspalten und der Bereich einer Excel-Tabelle folgen | `region.write(df)` |
| Vergleich | `apply::verify`: abweichende Zellen als Tabelle | `region.verify(df)` |
| Exakte Auswertung | `lift::calculate` auf der Mappe mit eingesetzten Tabellen | `wb.evaluate({"Orders": df})` |
| Polars-Programm | `codegen::polars_program`: Daten laden, Formeln als Polars-Ausdrücke in Abhängigkeitsreihenfolge, am Ende Vergleich mit der Mappe. Übersetzt: Rechnen, Vergleiche, `IF`/`AND`/`OR`/`NOT`, `ROUND` (15 Stellen, kaufmännisch), `ABS`, `SQRT`, `INT`, `MOD`, `MIN`/`MAX`/`SUM`, Aggregate über Tabellenspalten, `VLOOKUP` (exakt), `INDEX`/`MATCH` (exakt), `SUMIF`/`COUNTIF`/`AVERAGEIF` (Gleichheit), Text (`&`, `LEFT`, `RIGHT`, `MID`, `LEN`, `UPPER`, `LOWER`, `TRIM`). Alles andere liefert der Rechenkern | `wb.to_polars_code(path, accuracy="auto" | "plain" | "exact")`; `auto` führt das Programm aus und überlässt dem Rechenkern, was in Polars nicht den Wert der Mappe ergibt |
| Speicher | `store`: Verzeichnis mit `model.json` (Regionen, Typen, Logik), `workbook.xlsx` und `data/<Region>.parquet`. Eine Tabelle ist eine Datei mit ihren Spalten, eine Gruppe von Einzelzellen eine mit einer Zeile je Zelle (`long`, siehe unten). `store::run` setzt Daten und geänderte Logik ein, rechnet und schreibt alle drei neu | `wb.to_store(pfad)`, `xo.open_store(pfad)`, `store.read`, `.write`, `.values`, `.set_values`, `.set_logic`, `.run`, `.workbook()` |
| Logik bei den Daten | Notiz in jeder Parquet-Datei (Formel und Plan je berechneter Spalte/Zelle); `store::recalculate` rechnet aus den Dateien allein neu (siehe unten) | `store.recalculate()`, `store.notes`, `xo.recalculate(pfade)`, `xo.read_note(pfad)` |
| Zeilen ändern, Ausschnitte, Rückgängig | `view` (Zeilen als JSON, Filter und Sortierung, geänderte Zeilen), `store::patch`, `store::remember` / `undo`, `store::tables` (was die Notizen sagen); auch für einen Ordner mit Parquet-Dateien ohne Mappe | `store.patch`, `store.undo`, `store.tables`, `store.set_cells`, `store.rename` |
| MCP | `xl-mcp`: über stdin/stdout für Agenten auf demselben Rechner, über HTTP für entfernte (Copilot Studio, ChatGPT); Werkzeuge `list_workbooks`, `open_workbook`, `describe`, `read_region`, `change_rows`, `write_region`, `append_rows`, `set_value`, `add_sheet`, `set_logic`, `rename_region`, `run`, `recalculate`, `undo`, `read_range`, `verify`, `export_workbook`, `export_region`, `polars_code`, `export_port`, `port_guide` | – |
| Portierung | die Mappe als Python-Projekt auf Polars, das sich gegen die Mappe prüft (`xl-engine/src/port.rs`, Handbuch Abschnitt 8); Anleitung für Agenten in `skills/excel-port` | Enron-Mappe: 26 von 32 eigenständig, 0 abweichend; ein Agent (Claude Code) hat den Rest portiert: 32 von 32, auch auf veränderten Daten gleich dem Rechenkern |
| App | Daten, Modell und Mappe eines Speichers: im Browser (`xl-mcp --app <Ordner>`), im Chat (MCP App, Werkzeug `show_app`) und auf dem Desktop (`app/desktop`, Flutter, Engine eingebaut). Ein gemeinsamer Kern in Dart (`app/core`), zwei Oberflächen, die ihn zeichnen (`app/web` mit Jaspr, `app/desktop`) | – |

Das Benutzerhandbuch mit allen Aufrufen steht in [HANDBUCH.md](HANDBUCH.md).

### Die Mappe bleibt gebaut, wie sie war (`rows.rs`, `apply.rs`, `regions::regions_like`)

Ziel des Rückwegs: Wer in den Daten Werte ändert oder Zeilen dazu- und wegnimmt, bekommt die Mappe in ihrem alten
Aufbau zurück, damit weiterläuft, was in Excel darauf aufsetzt.

- `rows::move_rows` verschiebt die Zeilen eines Blatts ab einer Zeile, ganz oder nur in einigen Spalten: Zellen,
  verbundene Zellen, Zeilenhöhen, Anker von Bildern und Diagrammen; dazu alles, was Zellen nennt: Formeln aller Blätter,
  definierte Namen, Matrixformeln, Filter, Gültigkeitsregeln, Verknüpfungen, bedingte Formate, Kommentare, die
  Bereiche von Excel-Tabellen. `rows::end_ranges` lässt Bereiche, die an der letzten Zeile einer Tabelle enden, ihrer
  Länge folgen.
- `apply::write_regions` macht damit Platz: mehr Zeilen schieben nach unten, was unter der Tabelle liegt; weniger Zeilen
  holen es wieder hoch. Ganze Zeilen, wo das geht; nur die Spalten der Tabelle, wo daneben etwas über ihr Ende
  hinwegläuft und keine Zeile darunter zerrissen würde.
- `regions::regions_like` liest die Mappe mit dem, was der Speicher von ihr weiß: Eine Region, die dort beginnt, wo eine
  bekannte beginnt, behält deren Namen und bleibt Tabelle, auch mit einer Zeile. Ohne das hingen die Namen an der
  Reihenfolge und gingen beim Verschieben auf andere Regionen über.
- `store::run` lehnt ab, wenn mit den neuen Zeilen Regionen verschwänden, und lässt den Speicher, wie er ist.

Gemessen (`cargo run --release -p xl-polars --example shape_check corpus/src-enron 40 corpus/lo-oracle 3`): je Mappe bis
zu drei Tabellen, jede bekommt zwei Zeilen dazu, dann wieder weg, dann eine weniger.

| Schritt | Tabellen |
| --- | --- |
| Mappen / Tabellen | 501 / 743 |
| zwei Zeilen mehr: Regionen dieselben, Tabelle zwei Zeilen länger | 729 (98,1 %) |
| zwei Zeilen mehr: abgelehnt, weil Regionen verschwänden, oder nicht schreibbar | 14 |
| und zurück: die Mappe Zelle für Zelle wie vorher | 728 von 729 |
| und zurück: nicht wie vorher | 1 |
| eine Zeile weniger: Regionen dieselben | 706 von 729 |
| eine Zeile weniger: Regionen andere, oder nicht schreibbar | 23 (abgelehnt) |

Die Ablehnungen sind Platzkonflikte: Die Tabelle stieße beim Wachsen an das, was neben oder unter ihr liegt, oder das
Verschieben risse etwas Danebenliegendes auseinander. Beim Schrumpfen waren die drei angesehenen Fälle sehr kleine oder
nur aus Text bestehende Blöcke, die die Erkennung als Tabelle liest; die übrigen 20 habe ich nicht einzeln geprüft. Der
eine Fall, der nicht zurückkam: Dort wanderten beim Wachsen nur die Spalten der Tabelle, und beim Schrumpfen hätte
dasselbe eine Zeile zerrissen, also blieben die Zeilen stehen.

In Excel geprüft (`tools/excel_shape.py`): Eine Mappe mit Excel-Tabelle, Summe darunter, bedingtem Format,
Gültigkeitsregel, definiertem Namen, zweiter Tabelle und einem zweiten Blatt, um drei Zeilen gewachsen, öffnet ohne
Reparatur; Excels eigene Rechnung gibt dieselben Werte, und Tabelle, Format, Regel und Name liegen auf den neuen Zeilen.

### MCP-Server für entfernte Agenten

    xl-mcp --http 127.0.0.1:8787 --workspace <Ordner> --token <Geheimnis, mind. 16 Zeichen> [--public-url https://…]

- `POST /mcp/<Geheimnis>`: eine JSON-RPC-Nachricht, Antwort als JSON (Streamable HTTP ohne SSE).
- Der Agent sieht keine Dateien des Rechners: Mappen liegen im Arbeitsordner und werden beim Namen genannt; Pfade nach
  außen werden abgelehnt. `GET /files/<Geheimnis>` ist eine Seite zum Hoch- und Herunterladen, `PUT`/`GET
  /files/<Geheimnis>/<Name>` dasselbe für Programme; `export_workbook` nennt den Download-Link (mit `--public-url` den
  von außen erreichbaren).
- Logik schreibt man mit `[@Spalte]`, `Tabelle[Spalte]` und Zellnamen, in denen Leerzeichen und Zeichen `_` sind
  (`Tax rate` → `Tax_rate`); `set_logic` prüft sofort. `run` nennt die Zellen ohne Wert, `verify` sagt, ob seit dem letzten
  `run` etwas geändert wurde.
- Geprüft mit einem Agenten, der nur die Auskünfte des Servers kannte, an einer gebauten und einer Enron-Mappe: Im ersten
  Lauf 22 Stolperstellen (u. a. vier Fehlversuche bei einer Logikänderung); nach den Korrekturen im zweiten Lauf 17
  Aufrufe ohne Fehlversuch. Offen aus diesen Läufen: Regionsnamen wie `1_2`, Tabellen mit Kopfzeile in der zweiten Zeile,
  breite Einzelzellen-Regionen, die eigentlich Tabellen sind.
- Für Copilot Studio: Server starten, mit `cloudflared tunnel --url http://localhost:8787` eine HTTPS-Adresse holen,
  diese als `--public-url` angeben und im Agenten als MCP-Werkzeug `https://…/mcp/<Geheimnis>` eintragen
  (Anweisungen für den Agenten: `docs/copilot-agent.md`).

### Einzelzellen im Speicher: eine Zeile je Zelle (`long.rs`)

Eine Gruppe von Einzelzellen (Eingaben und Ergebnisse eines Modells) ist im Speicher eine Tabelle mit einer Zeile je
Zelle, nicht eine Zeile mit einer Spalte je Zelle:

| Spalte | Typ | Inhalt |
| --- | --- | --- |
| `name` | String | Name der Zelle |
| `cell` | String | Adresse (`B7`) |
| `dtype` | String | `Int64`, `Float64`, `Boolean`, `String`, `Date`, `Datetime`, `Time`, `Duration`; `Null` für eine Zelle ohne Wert |
| `number` | Float64 | Wert einer Zahl; eines Wahrheitswerts 1 oder 0 |
| `text` | String | Wert eines Textes; Datum und Zeit als Text (`2026-01-31`, `12:30:00`) |

- **Lesen**: `store.read("Model")` gibt diese Zeilen, `store.values("Model")` ein dict Name → Wert (Datum als Datum).
- **Schreiben**: `store.write("Model", pl.DataFrame({"name": ["Tax rate"], "number": [0.07]}))` ersetzt die genannten
  Zeilen, die übrigen Zellen behalten ihren Wert. `store.set_values("Model", {"Tax rate": 0.07})` tut dasselbe; eine
  Zeile mit einer Spalte je Zelle wird weiter angenommen. Ein unbekannter Name ist ein Fehler, der die vorhandenen nennt.
- **Was eine Zeile hält**, bestimmt `dtype`, wo es passt (ein Text in einer Zeile mit `Date` ist ein Datum), sonst der
  Ort des Werts: `number` ist eine Zahl, `text` ein Text.
- **MCP**: `read_region` gibt je Zelle `{"name", "cell", "value"}` und blättert mit `offset`/`limit`; `write_region` nimmt
  solche Zeilen (oder eine Zeile mit einem Feld je Zelle), `set_value` eine Zelle.
- Innerhalb der Engine (Regionen, Rückschreiben, Pläne, Polars-Programm, `region.to_polars()`) bleibt eine Gruppe von
  Einzelzellen eine Zeile mit einer Spalte je Zelle; `store::read_wide` gibt die Daten des Speichers in dieser Form.

An der Mappe mit 15.178 berechneten Einzelzellen in 39 Regionen: Parquet-Dateien 14,7 MB statt 37 MB (davon 14,4 MB die
Notizen), Neurechnen aus den Notizen samt Lesen und Schreiben 1,4 s mit dem Polars der Umgebung und 1,2 s mit dem
eingebauten (vorher 2,0 s und 14 s).

### Ausnahmezeilen: Zeilen, die nicht sind wie ihre Spalte

Eine Spalte hat einen Hauptzustand und eine Liste von Ausnahmen (`RegionColumn::exceptions`, Python
`column.exceptions`, im Modell und in der Notiz `"exceptions"`):

- Die Spalte ist **eine heruntergezogene Formel**, wenn dieselbe Formel in mindestens zwei Zeilen und in mindestens zwei
  Dritteln der Zeilen steht. Die übrigen Zeilen sind Ausnahmen: mit eigener Formel oder als fester Wert (eine
  überschriebene Zelle, eine leere Zeile).
- Die Spalte ist **Werte**, wenn mindestens zwei Drittel der Zeilen Werte sind. Zeilen mit Formel sind Ausnahmen.
- Sonst ist die Spalte keins von beidem, und der Block zerfällt wie bisher in Einzelzellen. In einer Excel-Tabelle
  (Listenobjekt) gilt eine solche Spalte als Werte, jede Formel darin als Ausnahme.
- Eine letzte Zeile, deren Werte alle Ausnahmen wären, ist eine Summenzeile und bleibt wie bisher eine eigene Region.

Was mit Ausnahmen geschieht:

| Wo | Verhalten |
| --- | --- |
| Rückschreiben (`apply`) | Eine Zeile mit eigener Formel behält sie. Ein fester Wert in einer berechneten Spalte wird aus der Tabelle geschrieben; steht dort kein Wert, bleibt die Zelle, wie sie ist. Neue Zeilen bekommen die Formel der Spalte |
| Logik ändern (`set_logic`, `model.json`) | Gilt für die Zeilen der Spalte, die keine Ausnahmen sind. Die Ausnahmen stehen im Modell zur Ansicht und werden nicht zurückgelesen |
| Notiz | Je Ausnahme `row` (ab 0 in den Daten) und, bei eigener Formel, `formula`, `logic`, `plan`, `inputs` wie bei einer Einzelzelle |
| Neurechnen aus den Notizen | Erst die Spalte, dann die Ausnahmen: ein fester Wert bleibt, eine eigene Formel wird gerechnet. Der Bericht nennt solche Zeilen `Spalte (row n)` |
| Polars-Programm | Spalten mit Ausnahmen liefert der Rechenkern (`wb.evaluate`) |
| MCP `describe` | Listet unter der Spalte die abweichenden Zeilen (`but D5: = …`) |

Hat eine berechnete Spalte als Ganzes keinen Plan (sie liest Zeilen über oder unter sich, die eigene Spalte, Text als
Zahl), aber jede Zeile für sich einen, stehen die Pläne der Zeilen unter `"each"`, und die Spalte wird Zeile für Zeile
von oben nach unten gerechnet. Liest die Formel einer Spalte mit Ausnahmen die eigene Spalte (Fortschreibung
`=A17+1`), gilt ihr Plan als Ganzes nicht, weil er jede Zeile als Fortsetzung der vorigen nimmt.

Ausnahmen hängen an der Zeilennummer, nicht an einem Schlüssel: Wer die Zeilen einer Tabelle umsortiert oder oben
einfügt, verschiebt die Daten unter den Ausnahmen hindurch.

### Zweizeilige Kopfzeilen und Stapel gleich gebauter Blätter (`regions.rs`, `stack.rs`)

- **Kopfzeile in zwei Zeilen** (`Region::header_rows`): oben die Gruppe, darunter die Spalte. Die zweite Zeile zählt zum
  Kopf, wenn sie nur Beschriftungen enthält und unter mindestens zwei davon Werte stehen und kein Text; eine einzelne
  Beschriftung über einem Teil der Spalten (verbunden, oder nicht ganz links) ist Gruppe, kein Titel. Der Spaltenname
  ist beides zusammen (`MW VOLUME Per EnPower`), eine verbundene Zelle gilt für alle Spalten unter ihr
  (`frame::frame_headed`). Vorher wurde die zweite Zeile als Daten gelesen: Zahlenspalten waren Text, Namen
  `MW VOLUME_1`, `column_1`.
- **Stapel** (`stack::stacks`): Tabellen in verschiedenen Blättern an derselben Stelle, mit denselben Spaltennamen und
  derselben Art von Daten je Spalte, sind zusätzlich zu ihren Regionen ein Stapel: eine Tabelle mit der Spalte `sheet`
  vorn. Die Logik darf je Blatt abweichen (`differs` nennt die Spalten). Ein Stapel hat keine eigene Datei; `store::
  read_region` und `write_frame` nehmen seinen Namen und lesen und schreiben die Dateien seiner Blätter, `set_model_logic`
  setzt die Spalte in allen. `store::add_sheet` legt ein Blatt als Kopie eines vorhandenen an (`copy_sheet`: Zellen,
  Formate, Verbünde, Breiten, Gültigkeit, bedingte Formate; keine Excel-Tabellen, Bilder, Diagramme). Python:
  `store.stacks`, `store.read("stack")`, `store.write`, `store.add_sheet`; MCP: Stapel in `describe`, Werkzeug `add_sheet`.
- **Fester Wert in berechneter Spalte:** Wird eine Tabelle ohne die berechnete Spalte neu geschrieben, bleibt der Wert in
  seiner Zeile (`Data::nulls_keeping`); vorher rechnete `recalculate` ohne ihn, `run` mit ihm.
- **Aufrufprotokoll:** `xl-mcp --log DATEI` schreibt jeden Werkzeugaufruf als JSON-Zeile.
- Messung nach der Kopfzeilen-Änderung (501 Mappen): 82.348 von 98.432 Zellen neu gerechnet (83,7 %, vorher 82.614),
  0 abweichend. Die 266 Zellen weniger sind nicht untersucht.
- Geprüft mit Claude Desktop über MCP (stdio), drei Aufgaben an einer gebauten und einer Enron-Mappe, vor diesen
  Änderungen: alle richtig gelöst; der Agent benannte die zweizeilige Kopfzeile als Schwäche. Handbuch:
  `docs/HANDBUCH.md`.

### Die Logik reist mit den Daten (`plans.rs`, Notizen in den Parquet-Dateien)

Jede Parquet-Datei eines Speichers trägt in ihren Datei-Metadaten (Schlüssel `xlcells`, JSON) eine Notiz: welche Region
sie ist (Blatt, Bereich, Art) und je berechneter Spalte oder Zelle die Formel, die strukturierte Schreibweise und, wo der
Rechenkern einen hat, seinen Plan über die Daten der Regionen. Die Notiz steht auf Dateiebene und nicht je Feld, weil
Polars Metadaten je Feld nicht zurückliest; so lesen sie Polars (`pl.read_parquet_metadata`), pyarrow und DuckDB.

    {"version": 1, "region": "Sales", "kind": "table", "sheet": "Sales", "ref": "A1:E4", "date1904": false,
     "columns": [
       {"name": "qty", "dtype": "Int64"},
       {"name": "amount", "dtype": "Float64", "formula": "ROUND(B2*C2*(1+Model!$B$1),2)",
        "logic": "ROUND([@qty]*[@price]*(1+Tax_rate),2)", "plan": 0, "first_row": 2, "rows": 3,
        "inputs": [{"name": "i0", "column": "qty"}, {"name": "i1", "column": "price"},
                   {"name": "i2", "cell": ["Model", "Tax rate"]}]},
       {"name": "today", "dtype": "Date", "formula": "TODAY()", "why": "volatile function"}],
     "plans": [{"nodes": [["col", "i0"], …], "steps": [], "outputs": [["_r", 12], ["_rt", 15]], "broadcast": true}]}

- `plans` hält jeden Plan einmal (Form: `ir::wire`, Standard-JSON; `wire::plan_from_json` liest ihn zurück). Die Eingaben
  heißen im Plan `i0`, `i1`, …; dieselbe Formel an anderer Stelle ist derselbe Plan. Eine Mappe mit 15.178 berechneten
  Einzelzellen hat 25 verschiedene Pläne.
- `inputs` sagt, wo jede Eingabe in den Daten liegt: Spalte einer Region, benannte Zelle (Name in der eigenen Region, sonst
  `[Region, Name]`), Zelle einer Tabelle, die Zahlen eines Bereichs (`range`: Tabellenspalten oder Zellen) oder ein Wert,
  der keine Daten ist (Zelle außerhalb aller Regionen, mit ihrer Adresse); `as`, als was sie gelesen wird, wenn nicht als Zahl.
- `why` steht, wo es keinen Plan gibt.
- Die Notiz einer Gruppe von Einzelzellen hat `"form": "rows"`: Die Datei hat eine Zeile je Zelle (siehe oben), und
  `columns` der Notiz sind die Zellen, mit `cell` als Adresse.
- `exceptions` an einer Spalte: die Zeilen, die nicht sind wie die Spalte (`row` ab 0 in den Daten), bei eigener Formel
  mit `formula`, `logic`, `plan`, `inputs`; ohne Formel ein fester Wert. `each`: Pläne je Zeile für eine Spalte, die als
  Ganzes keinen hat.
- Eine Zahl in einer Spalte gemischter Art (Zahlen und Text) ist in den Daten Text; `{"table": [Region, Spalte, Zeile],
  "parse": true}` liest sie aus diesem Text zurück (Zahlen exakt, Datum als Seriennummer).

`plans::Mapper` erzeugt diese Zuordnung; das Polars-Programm (`codegen`) schreibt dieselbe Zuordnung als Python aus.

**Neu rechnen ohne Mappe**: `store::recalculate(dir)` (Python `store.recalculate()`; für lose Dateien
`xo.recalculate([pfade])`) liest die Notizen, führt die Pläne in der Reihenfolge der Abhängigkeiten mit Polars aus und
schreibt die Dateien mit ihren Notizen neu. Eine Tabelle darf dabei mehr oder weniger Zeilen haben als beim Export. Das
Ergebnis nennt, was gerechnet wurde (`done`), was seine Werte behalten hat und warum (`kept`: kein Plan, oder die Daten
haben nicht mehr die Art, für die er gemacht wurde, etwa Text statt Zahlen) und was zwar gerechnet wurde, aber aus etwas
Behaltenem (`from_kept`). Zeilen, die einzeln gerechnet werden (Ausnahmen, Spalten mit `each`), stehen dort einzeln als
`Spalte (row n)`; „aus Behaltenem“ gilt je Zeile: Eine behaltene Zeile färbt nur, was genau diese Zeile oder die ganze
Spalte liest. Die Mappe bleibt unberührt; `store.run` zieht sie nach und rechnet dabei alles mit dem Rechenkern.

Schreibt ein anderes Werkzeug eine Parquet-Datei neu, ist die Notiz meist weg. Im Speicher liegt sie deshalb noch einmal
in `notes.json`; außerhalb gibt man sie beim Schreiben mit:
`df.write_parquet(pfad, metadata={xo.NOTE_KEY: json.dumps(xo.read_note(pfad))})`.

Gemessen (`cargo run --release -p xl-polars --example notes_check corpus/src-enron 40 corpus/lo-oracle 3`, 501 Mappen):
Je Mappe wird ein Speicher angelegt, alle nicht berechneten Zahlen der Parquet-Dateien werden mit 1,37 multipliziert, die
Dateien werden aus ihren Notizen neu gerechnet und mit der Mappe verglichen, die der Rechenkern mit denselben Daten rechnet.

| | Zellen | |
| --- | ---: | --- |
| berechnete Zellen in Regionen | 98.432 | davon 1.110 Ausnahmezeilen mit eigener Formel |
| aus den Notizen neu gerechnet | 82.614 | 83,9 % |
| davon mit der Mappe verglichen | 74.498 | alle gleich, 0 abweichend |
| davon aus etwas Behaltenem gerechnet (`from_kept`) | 2.160 | 1.848 gleich, 312 nicht: erwartet, die Quelle ist auf altem Stand |
| davon ohne Vergleich | 5.956 | der Rechenkern der Mappe hat die Zelle selbst nicht gerechnet |
| Werte behalten | 15.818 | 16,1 % |

Gründe für behaltene Werte (nach Art zusammengefasst, `REASONS=60`): Der Wert einer Zelle hat die Übersetzung geformt
(7.548: Nachschlagetabellen von `MATCH`, `VLOOKUP`, `OFFSET`, Bedingungsbereiche von `COUNTIF`/`SUMIF`, und Zellen, die
einen Fehlerwert oder eine nicht gerechnete Zelle lesen), Funktionen aus Add-ins und DDE-Verknüpfungen (3.283), Zeilen,
für die die Übersetzung nicht gilt (1.242), Daten anderer Art als beim Export (858: eine Zahlenspalte ist durch Zeilen
mit Text zu Text geworden), Kreis auf Spaltenebene (791), `DAYS360` (421), flüchtige Funktionen (313). Die Abhängigkeit
von der Blattlänge (vorher 4.584) ist kein Grund mehr: Die Übersetzung liest die Blattlänge für einen Nachschlageblock
nur noch dort, wo sie sie braucht; diese Zellen stehen jetzt unter dem eigentlichen Grund, der eingesetzten Tabelle.

Mit den Ausnahmezeilen liegen in dieser Stichprobe mehr Formelzellen in Tabellen statt in Einzelzellen; der Anteil des
neu Gerechneten ist derselbe wie davor (82.611), weil eine Spalte ohne Plan als Ganzes Zeile für Zeile gerechnet wird.

Zeiten an einer Mappe mit 15.178 berechneten Einzelzellen in 39 Regionen (`store_time`): Notizen erzeugen 3,1 s, neu
rechnen 0,6 s (Einzelzellen mit demselben Plan gehen gemeinsam durch Polars; samt Lesen und Schreiben 1,4 s). Der Rechenkern braucht für dieselbe Mappe über `store.run` 22 s. Die Notizen dieser Mappe sind 14 MB
groß, fast ganz Namen und Formeln der 15.178 Zellen.

Grenzen:
- Was aus etwas Behaltenem gerechnet wird, ist nur richtig, wenn sich das Behaltene nicht geändert hätte.
- Die Reihenfolge des Neurechnens gilt je Spalte. Lesen zwei Spalten einander in verschiedenen Zeilen (Gittermodelle,
  die als Tabelle erkannt sind), ist das hier ein Kreis, und beide behalten ihre Werte; die Mappe rechnet sie.
- Eine Spalte, die Zeile für Zeile gerechnet wird (`each`), hat für neue Zeilen keinen Plan; sie stehen unter `kept`.
- Ein Bereich über mehr als 400 Zellen, der nur einen Teil einer Tabelle trifft, hat keinen Plan.
- Das Neurechnen kennt die Mappe nicht: Für behaltene Spalten bleibt `store.run` der Weg zum richtigen Wert.

### Messung an echten Mappen (Enron, die ursprünglichen `.xls`)

`cargo run --release -p xl-polars --example migrate_stats corpus/src-enron 100` (jede 100. Datei: 194 gelesen,
10 unlesbar, 5 über 200.000 Zellen, kein Absturz):

| Größe | Wert |
| --- | --- |
| Regionen | 1.911 Tabellen, 12.890 Gruppen von Einzelzellen, 10.516 Blöcke |
| Formelzellen | 159.810; davon 18.414 (11,5 %) in berechneten Spalten, 141.396 als Einzelzellen |
| berechnete Spalten und Zellen | 141.570, davon 96.051 (67,8 %) mit Polars-Übersetzung |
| Rückschreiben der eigenen Tabellen aller Regionen | 25.282 Regionen, 0 geänderte Zellen |

Die Tabelle zeigt den Stand vor den Ausnahmezeilen. Mit ihnen, an jeder 40. Datei gemessen (489 gelesen):

| Größe | ohne Ausnahmezeilen | mit Ausnahmezeilen |
| --- | --- | --- |
| Tabellen / Gruppen von Einzelzellen | 4.190 / 37.647 | 4.567 / 37.361 |
| Formelzellen in Spalten (von 632.358) | 46.223 (7,3 %) | 199.440 (31,5 %) |
| Ausnahmezeilen | – | 3.806 in 1.330 Spalten |
| berechnete Spalten und Einzelzellen | 586.456, davon 73,0 % mit Polars-Übersetzung | 434.634, davon 73,3 % |
| Rückschreiben der eigenen Tabellen | 57.940 Regionen, 0 geänderte Zellen | 58.031 Regionen, 0 geänderte Zellen |

An den LibreOffice-Testmappen (jede 3., 151 gelesen) steigen die Formelzellen in Spalten von 1.826 auf 5.210 (von 24.287),
mit 791 Ausnahmezeilen in 161 Spalten.

Die Mappen sind überwiegend Modelle in Gitterform (Formeln quer über Zeilen, Zwischensummen in Tabellen), keine
Spaltentabellen; daher der hohe Anteil Einzelzellen. Häufigste Gründe ohne Polars-Übersetzung: Bereiche, die keine ganzen
Tabellenspalten sind (17.606), andere Mappen (6.517), Funktionen ohne Übersetzung (`EOMONTH`, `ISERROR`, `RANK`, `MONTH`,
benutzerdefinierte wie `XSPRDOPT`).

`tools/polars_code_check.py corpus/src-enron 400` führt das erzeugte Programm aus und vergleicht jeden Wert mit dem von
Excel gespeicherten (43 Mappen bis 20.000 Zellen, 19 mit Formeln; 54 s): mit `plain` laufen 18 Programme, 5.106 von 5.112 in
Polars geschriebenen Spalten und Zellen treffen jeden Wert; ein Programm scheitert an einer Zeile (Text plus Zahl).
`auto` prüft jede Spalte und Zelle gleich nach ihrer Berechnung und nimmt bei Fehler oder Abweichung den Wert des
Rechenkerns: alle 19 Programme laufen, 10.164 von 13.670
Spalten und Zellen (74 %) stehen in Polars und treffen jeden Wert; in 3 Programmen weichen Werte ab, die der Rechenkern
liefert (Fehlerwerte, andere Mappen, benutzerdefinierte Funktionen).

Grenzen dieser ersten Fassung:

- Erkennung, Rückschreiben und Programm sind an selbst gebauten Mappen und an der Enron-Stichprobe geprüft, nicht an Excel
  selbst. Speicher und MCP-Server sind nur an selbst gebauten Mappen geprüft.
- Modelle in Gitterform werden zu sehr vielen Einzelzellen; Tabellen, deren Formeln
  quer über die Zeile laufen, werden nicht als Tabellen erkannt. Zwischensummen und überschriebene Zellen sind
  Ausnahmezeilen, solange zwei Drittel der Spalte einheitlich sind.
- Eine Tabelle wächst nur in freie Zeilen; was darunter steht, wird nicht verschoben (Fehlermeldung nennt die Zelle).
- Im Modell lassen sich Logik und Daten ändern, nicht aber Regionen, Spalten oder Zellen hinzufügen oder umbenennen.
- Das Polars-Programm rechnet Einzelwerte wie Excel (leer zählt 0, Datum als Seriennummer, Bereiche nehmen nur Zahlen),
  aber im 1900-Datumssystem und ohne Excels Fehlerwerte.
- Der Rechenkern stimmt mit Excel auf 15 Stellen überein, nicht immer im letzten Bit: Polars teilt eine Spalte durch eine
  Konstante über den Kehrwert (`ROUND(7.14…;2)` ergibt 7,140000000000001).
- `xl-mcp` über HTTP ist nur durch das Geheimnis in der Adresse geschützt (kein OAuth) und für Tests gedacht; mit Copilot
  Studio selbst ist es noch nicht gelaufen.

## Aussehen einer Zelle (`crates/xl-look`)

Excel setzt das Aussehen einer Zelle aus Schichten zusammen; die Datei speichert die Schichten, nicht das Ergebnis.
`xl-look` rechnet das Ergebnis aus, Attribut für Attribut, und sagt zu jedem, woher es kommt:

1. Vorgabe der Mappe (Formatvorlage `Normal`) — Quelle `default`
2. für eine Zelle ohne eigenen Eintrag das Format ihrer Zeile (`customFormat`, ohne `s` ist es Format 0), sonst ihrer Spalte — `row`, `column`
3. Tabellenformat, wo die Zelle in einer Tabelle liegt, nur für Attribute, die das Zellformat auf der Vorgabe lässt —
   `table` (Tabelle, Format, Element wie `headerRow`, `firstRowStripe`); Reihenfolge der Elemente nach ECMA-376 §18.8.41,
   äußere Rahmen am Rand des Elementbereichs, innere (`vertical`, `horizontal`) dazwischen
4. das Zellformat, das Excel vollständig speichert, mit seiner Formatvorlage: ein Attribut, das dem der Vorlage gleicht,
   kommt aus der Vorlage — `style` (Name), sonst `cell`
5. bedingte Formatierung nach Priorität, mit `stopIfTrue` — `conditional` (Bereich, Priorität, Regeltyp)

Attribute: Schriftart, -größe, fett, kursiv, unterstrichen, durchgestrichen, Schriftfarbe, hoch/tief; Füllmuster, Füllfarbe,
Hintergrundfarbe des Musters; Rahmen links, rechts, oben, unten, diagonal mit Farbe; horizontale und vertikale Ausrichtung,
Umbruch, Drehung, Einzug, Verkleinern; Zahlenformat; gesperrt, ausgeblendet. Dazu Datenbalken (Farbe, Anteil) und
Symbolsatz (Satz, Symbol). Farben als `RRGGBB`, Designfarben mit Farbton (`tint`) und Palettenindizes (auch eigene Palette)
aufgelöst; die angegebene Form steht daneben (`theme 4 tint 0.6`).

- **Formelregeln** (`expression`, `cellIs` und die Text-, Leer-, Fehler- und Datumsregeln, für die Excel die Formel
  mitspeichert): Die Formel wird auf die Zelle verschoben, wie Excel sie füllt, ihre Bezüge mit dem Blatt qualifiziert, in
  ein Hilfsblatt geschrieben und von `xl-polars` berechnet, über den Werten, die die Datei hält (Formelzellen mit
  gespeichertem Wert gehen als dieser Wert ein, wie bei den Bereichsregeln). `ROW()` und `COLUMN()` ohne Argument
  meinen die Zelle, `TODAY()` und `NOW()` den Zeitpunkt `Looker::now` (Vorgabe: jetzt, Ortszeit wie in Excel). Eine Formel mit
  Fehlerergebnis wendet nichts an. Was die Engine nicht verbürgt, bleibt unentschieden: das Attribut wird als
  `certain = false` markiert, nichts geraten (im Korpus noch zwei Zellen: `ISEVEN` fehlt der Engine).
- **Bereichsregeln** (Top/Bottom, über/unter dem Mittel, Duplikate/eindeutige, Farbskalen, Datenbalken, Symbolsätze)
  werden aus den Zellwerten berechnet.
- **Tabellenformate**: die 60 eingebauten Tabellen- und 84 PivotTable-Formate aus ECMA-376, wie Apache POI sie ausliefert
  (`crates/xl-look/data`, Apache-2.0, siehe `NOTICE`), und die eigenen der Datei (`<tableStyles>`, jetzt im Modell gelesen
  und geschrieben, ebenso Design und eigene Palette).

Rust: `Looker::new(&wb).looks(&[(blatt, zeile, spalte), …])`. Python: `ws["B3"].look.fill_color`,
`ws["B3"].look.fill_color_source` (`.kind`, `.name`, `.detail`, `.certain`, `.spec`), viele Zellen auf einmal mit
`ws.looks("A1:F40")` (auch `"A:C"`, `"2:4"`), Zelle für Zelle auf demselben Stand mit `wb.looker().look(zelle)` (die Mappe
wird dafür einmal geschrieben und gelesen, nicht je Zelle).

### Absicherung

- **Gegen LibreOffice** (`tests/libreoffice.rs`, Daten von `tools/lo_look.py`): Für Stichprobenzellen jeder xlsx des Korpus
  die Zelleigenschaften, die LibreOffice nach dem Laden hat (Formatvorlage, Zellformat, Designfarben aufgelöst), und die
  Hintergrund- und Schriftfarben seines HTML-Exports, in dem die bedingte Formatierung angewandt ist. Stand: 1.466
  Arbeitsmappen, 76.039 Zellen mit je 24 Attributen (Schrift, Füllung, vier Rahmen mit Farbe, Ausrichtung, Umbruch,
  Drehung, Einzug, Schutz, `General`), dazu 2.904 Füllfarben aus dem HTML-Export, davon 228 von bedingten Formaten
  gesetzt, und 73 Schriftfarben bedingter Formate: **keine Abweichung**.
  Was LibreOffice anders macht als Excel, wird nicht verglichen und mit Namen gezählt: eigene Vorgabeschrift, wenn das
  Stylesheet keine Schrift liefert; Rahmen der Nachbarzelle auf der gemeinsamen Kante; Format der Spalte für eine Zelle
  ohne `s`; Musterfüllungen, die es zu einer Farbe mischt; Farben der eigenen Palette mit Alpha `FF`, die es als
  durchsichtig liest; `BOOLEAN` als Zahlenformat logischer Zellen; übersetztes `General`; Umbruch bei Blocksatz;
  bedingte Formate mehrerer Bereiche über einer Zelle (LibreOffice ordnet nach Bereich, Excel nach Priorität über das
  ganze Blatt); PivotTables (LibreOffice legt sie selbst neu an).
- **PivotTables**: Ihr Format hängt am Aufbau der PivotTable (Zeilen-, Spaltenfelder, Teil- und Gesamtergebnisse) und
  ist nicht umgesetzt. Zellen einer PivotTable (mit den Seitenfeldern darüber) tragen den Hinweis in `undecided`, und
  Schrift, Füllung und Rahmen sind als nicht sicher markiert (`xl_look::pivot::pivot_areas` liest die Lage aus der
  Datei).
- **Leere Zellen mit `s="0"`** in einer formatierten Zeile oder Spalte behält das Modell jetzt (sie haben Format 0, nicht
  das der Zeile oder Spalte) und schreibt sie mit `s="0"`.
- **Farbton**: ECMA-376 gibt die Formel, nicht die Rundung. Excel rechnet mit der ganzzahligen HLS-Umrechnung des
  Windows-SDK (`ColorRGBToHLS`/`ColorHLSToRGB`, HLSMAX 240, jede Division abgeschnitten), die neue Leuchtdichte
  abgeschnitten: gegen Excel selbst geprüft (93 Farben: Design-, Paletten- und RGB-Farben, Farbton −0,5 bis 0,8), alle
  gleich. LibreOffice rundet anders und liegt bis zu zwei Stufen je Kanal daneben; der LibreOffice-Vergleich lässt bei
  getönten Farben diese zwei Stufen zu.
- **Farbskalen**: linear je Kanal, abgeschnitten wie bei LibreOffice; gegen Excel geprüft (zwei- und dreifarbig, Grenzen
  als Minimum/Maximum, Zahl, Prozent, Perzentil), gleich.
- **Regeln nach Art**: Leer, nicht leer, Fehler, kein Fehler und Zeiträume (`timePeriod`) entscheidet Excel nach der Art
  der Regel, nicht nach der mitgespeicherten Formel (für einen Fehlerwert ist `LEN(TRIM(A1))>0` ein Fehler, „nicht leer“
  gilt trotzdem). `cond::by_type` tut dasselbe; Wochen laufen Sonntag bis Samstag.
- **Gegen Excel** (`tools/excel_look.py`, macOS mit Microsoft Excel): Eine Mappe mit Szenarien (Farbtöne, alle Regelarten,
  Priorität und `stopIfTrue`, Top/Bottom, Mittelwert, Duplikate, Farbskalen, Zeiträume, Verankerung von Formeln über
  mehrere Bereiche, Zeilen-, Spalten- und Formatvorlagen, Tabellenformate) wird in Excel geöffnet und je Zelle die
  angezeigte Füllung, Schriftfarbe, Fett und Kursiv gelesen (`display format`), jedes Szenario als eigene Mappe. Ergebnis
  (2026-11-15, 534 Zellen): Verankerung, Top/Bottom, Mittelwert, Duplikate, Farbskalen, Priorität und `stopIfTrue`
  stimmten von Anfang an; abgewichen hatten die Farbtöne (1–2 Stufen je Kanal), „nicht leer“ auf Fehlerwerten und die
  Zeiträume (unentschieden). Alle drei sind behoben; nach der Änderung liefen Regeln und Zeiträume gleich, die Farbtöne sind
  gegen die 93 von Excel gelesenen Werte geprüft. Schriftart, Größe, Rahmen und Ausrichtung liest das Werkzeug noch nicht.
- **Tabellenformate** (`tests/look.rs`): LibreOffice wendet sie nicht an; geprüft wird gegen die ECMA-Definitionen
  (TableStyleMedium9: Kopfzeile, Streifen, innere Rahmen, Vorrang des Zellformats).
- Offen: ob Excel bei `<c>` ohne `s` das Format der Spalte nimmt (LibreOffice tut es, die Spezifikation sagt Format 0);
  Schriftart aus dem Design bei `scheme="minor"`; die Prozentrundung bei Top-N-Prozent und die Standardabweichung bei
  „über dem Mittel“ (beide ohne Korpusfall).
