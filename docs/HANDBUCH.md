# excelinator: Handbuch und Schnittstellen

Stand: Zweig `regions-store-mcp`, 22.11.2026. Dieses Handbuch beschreibt den Weg von einer Excel-Mappe zu Tabellen in
Polars und zurück. Das Lese-/Schreibmodul selbst (die openpyxl-Fassade) und die Messungen stehen in der
[technischen Notizen](TECHNIK.md).

Die Ausgaben in den Beispielen stammen aus einem Lauf mit genau diesen Befehlen.

## 1. Wozu

Eine Mappe hält Daten und Rechnungen in Zellen. excelinator nimmt sie auseinander in

- **Tabellen** (Polars-DataFrames, gespeichert als Parquet),
- **Logik** (je berechneter Spalte oder Zelle eine Formel, lesbar geschrieben),
- **die Mappe als Form**, in die beides zurückgeht, mit Layout und Formaten.

Ein Agent, ein Programm oder ein Mensch in der Web-App ändert Daten und Logik an den Tabellen. Die Rechnungen laufen
ohne Excel weiter, und die Mappe lässt sich jederzeit wieder ausgeben: gebaut wie das Original, mit den Daten von jetzt.

Drei Ebenen, dieselben Daten:

| Ebene | Was man sieht | Womit |
|---|---|---|
| Mappe | Blätter als Zellraster, Formeln, die erkannten Regionen | Web-App (»Mappe«), `read_range`, `store.set_cells` |
| Modell | Regionen, Stapel, Logik je Spalte und Zelle, Stand der Überführung | Web-App (»Modell«), `describe`, `store.tables` |
| Daten | Tabellen als Parquet-Dateien mit ihrer Logik, auch ohne Mappe | Web-App (»Daten«), `read_region`, `change_rows`, `store.patch` |

## 2. Einrichten

Aus einem Release (ein Paket für jeden Prozessor; der Rechenkern ist WebAssembly, nichts wird gebaut):

    uv pip install xlcells-0.1.0.dev1-py3-none-any.whl       # die Bibliothek
    uv tool install xlcells-0.1.0.dev1-py3-none-any.whl      # der MCP-Server als Programm xlcells-mcp

Unter Windows ohne Installation: den Ordner `excelinator-…-windows-x64.zip` entpacken (siehe `docs/RELEASE.md`).

Aus dem Repository, für die Arbeit daran:

    uv sync                                    # im Wurzelverzeichnis: baut die Python-Pakete nach .venv
    cargo build --release -p xl-mcp            # der MCP-Server; er enthält auch die Web-App
    target/release/xl-mcp --app <Ordner>       # die Web-App: http://127.0.0.1:8787/

```python
import polars as pl
import xlcells.openpyxl as xo
```

`xlcells` ist das Lese-/Schreibmodul mit der openpyxl-Fassade. Alles, was rechnet oder Regionen findet, braucht
zusätzlich das Paket `xlcells_engine` (nutzt das Polars der Umgebung) oder `xlcells_engine_bundled` (bringt Polars mit).
pandas wird nicht bedient.

## 3. Der Weg an einem Beispiel

Die Mappe `model.xlsx` hat zwei Blätter:

- `Sales`: Kopfzeile `item, qty, price, amount`, vier Zeilen. `amount` ist `=ROUND(B2*C2*(1+Model!$B$1),2)`,
  heruntergezogen; in `D4` hat jemand die Formel mit dem Wert 99 überschrieben.
- `Model`: `Tax rate` 0,19; `Gross` `=SUM(Sales!D2:D5)`; `Today` `=TODAY()`.

### 3.1 Was die Mappe enthält

```python
wb = xo.load_workbook("model.xlsx")
print(wb.regions)
```

    <Region 'Sales' table Sales!A1:D5: item, qty, price, amount>
    <Region 'Model' cells Model!A1:B3: Tax rate, Gross, Today>

```python
wb.regions["Sales"].columns
```

    [<Column 'item' A String>, <Column 'qty' B Int64>, <Column 'price' C Int64>,
     <Column 'amount' D Float64 =ROUND([@qty]*[@price]*(1+Tax_rate),2) (1 exceptions)>]

Die Formel der Spalte steht in strukturierter Schreibweise da; `D4` ist als Ausnahme erkannt
(`<RowException D4 value>`). `wb.regions["Sales"].to_polars()` gibt die Tabelle als DataFrame.

### 3.2 Speicher anlegen

```python
store = wb.to_store("store")
print(store)
```

    <Store store>
      Sales (table Sales!A1:D5)
        item: String
        qty: Int64
        price: Int64
        amount: Float64  =ROUND([@qty]*[@price]*(1+Tax_rate),2)
      Model (cells Model!A1:B3)
        Tax rate: Float64
        Gross: Float64  =SUM(Sales[amount])
        Today: Null  =TODAY()

Auf der Platte:

    store/
      model.json            Regionen, Typen, Logik (zum Lesen und Ändern)
      workbook.xlsx         die Mappe als Form
      notes.json            Kopie der Notizen
      data/Sales.parquet    eine Datei je Region
      data/Model.parquet

### 3.3 Lesen

```python
store.read("Sales")      # Tabelle: ihre Spalten
store.read("Model")      # Einzelzellen: eine Zeile je Zelle
store.values("Model")    # Einzelzellen als dict
```

    ┌──────────┬──────┬─────────┬────────┬──────┐
    │ name     ┆ cell ┆ dtype   ┆ number ┆ text │
    ╞══════════╪══════╪═════════╪════════╪══════╡
    │ Tax rate ┆ B1   ┆ Float64 ┆ 0.19   ┆ null │
    │ Gross    ┆ B2   ┆ Float64 ┆ 115.66 ┆ null │
    │ Today    ┆ B3   ┆ Null    ┆ null   ┆ null │
    └──────────┴──────┴─────────┴────────┴──────┘
    {'Tax rate': 0.19, 'Gross': 115.66, 'Today': None}

### 3.4 Daten ändern und neu rechnen, ohne Mappe

```python
store.write("Sales", pl.DataFrame({
    "item": ["a", "b", "c", "d", "e"], "qty": [1, 2, 3, 4, 10], "price": [2.0, 2.0, 2.5, 1.0, 3.0]}))
store.set_values("Model", {"Tax rate": 0.07})
store.recalculate()
```

    {'done': [('Sales', 'amount'), ('Model', 'Gross')],
     'kept': [('Model', 'Today', 'TODAY()', 'volatile function: the value depends on the moment or on chance; …')],
     'from_kept': []}

`recalculate` rechnet aus den Parquet-Dateien und ihren Notizen allein. `amount` ist für fünf Zeilen neu gerechnet,
`Gross` danach. `Today` hat keinen Plan und behält seinen Wert, mit Grund.

### 3.5 Logik ändern und zurück in die Mappe

```python
store.set_logic("Sales", "amount", "[@qty]*[@price]*(1+Tax_rate)")
store.run()
```

    {'formula_cells': 6, 'computed': 5,
     'not_computed': {'volatile function: …': 1},
     'regions': {'Sales': 5, 'Model': 3}, 'logic_changed': [('Sales', 'amount')]}

`run` setzt Daten und Logik in die Mappe, rechnet jede Formel und schreibt Parquet, Modell und Mappe neu. In der Mappe
steht danach:

    D2 =B2*C2*(1+'Model'!$B$1)     D4 99     D6 =B6*C6*(1+'Model'!$B$1)     Model!B2 =SUM(Sales!D2:D6)

Die Tabelle ist um eine Zeile gewachsen, die Formel ist mitgezogen, die Summe folgt dem neuen Bereich, `D4` ist
geblieben. `store.workbook()` gibt die Mappe, `store.workbook().save("neu.xlsx")` schreibt sie.

## 4. Die App: im Browser, im Chat, auf dem Desktop

Eine App, drei Oberflächen. Was die App zeigt und tut, steht einmal im gemeinsamen Kern (`app/core`, Dart); jede
Oberfläche zeichnet nur.

| Oberfläche | Start | Weg zur Engine |
|---|---|---|
| Browser | `xl-mcp --app <Arbeitsordner> [--port 8787]`, dann die genannte Adresse | HTTP zum Server (`POST /api/…`) |
| Chat (MCP App) | der Agent ruft `show_app`; der Chat zeigt die App im Gespräch | über den Chat an das Werkzeug `app_call` |
| Desktop (Windows, macOS) | `flutter run -d macos` in `app/desktop` | eingebaut (`flutter_rust_bridge`), kein Server |

`xl-mcp --app` startet den Server nur für den eigenen Rechner (`127.0.0.1`, ohne Anmeldung). Die App ist Teil des
Programms; zum Bauen des Servers braucht es kein Dart (die gebaute App liegt in `crates/xl-mcp/web/`).
`<Adresse>/app/local/chat.html?store=<Name>.store` zeigt die App so, wie ein Chat sie einbettet: in einem
abgeschotteten Rahmen ohne eigenen Netzzugang. Die Desktop-App fragt beim ersten Start nach einem Arbeitsordner
(»Ordner …« wechselt ihn); was sie exportiert, zeigt sie im Finder oder Explorer.

Die App sieht aus wie Excel: oben die grüne Titelleiste mit dem Datensatz, darunter die drei Ebenen als Reiter und
rechts davon, was mit dem Datensatz geht; unten eine Statusleiste mit der letzten Meldung. Ist das System dunkel
eingestellt, ist es die App auch (im Chat folgt sie dem Chat).

Was folgt, gilt für alle drei. Abweichungen: Im Chat gibt es kein Hochladen und keine Adressen; auf dem Desktop fragt
die App in eigenen Fenstern statt auf der Seite.

**Start.** Die Excel-Dateien des Arbeitsordners; »überführen« legt den Speicher `<Datei>.store` an und rechnet ihn
einmal. »Mappe hinzufügen« lädt eine Datei in den Arbeitsordner. Oben wählt man den Datensatz; im Browser hat jede
Ansicht eine Adresse der Form `#/<Datensatz>/<Ebene>/<Tabelle oder Blatt>`, die sich als Lesezeichen oder Verweis eignet.

**Daten.** Links die Tabellen, Einzelzellen-Gruppen und Stapel, rechts das Raster.

- Berechnete Spalten sind blau hinterlegt und tragen `ƒ`; was nur die Mappe rechnen kann, ist gelb; eine Zeile, die
  nicht ist wie ihre Spalte (eigene Formel, fester Wert), rot.
- Doppelklick auf einen Wert ändert ihn. Danach rechnet die App alles Abhängige aus den Notizen neu, ohne Mappe.
- Ein Klick auf eine Spalte oder Zelle zeigt in der Zeile über dem Raster ihre Logik, was sie liest (als Verweis auf
  die andere Tabelle) und ob sie die Mappe braucht. Eine dort eingetragene Formel ist ihre neue Logik (wird geprüft
  und von der Mappe gerechnet).
- Unter jedem Spaltenkopf steht ein Filter: Text (»enthält«), oder `=`, `!=`, `<`, `<=`, `>`, `>=` mit einem Wert, oder
  `leer`. Klick auf den Kopf sortiert, Umschalt-Klick nach einer weiteren Spalte. »Suchen« findet in allen Spalten.
  Die Fußzeile summiert die Zahlenspalten des Ausschnitts.
- »Ausschnitt merken« speichert Filter und Sortierung unter einem Namen (links unter »Ausschnitte«).
- »+ Zeile«, Zeilennummer anklicken und »− Zeilen« ändern die Zeilenzahl. »CSV« schreibt den Ausschnitt, »Parquet«
  die Datei der Tabelle mit ihrer Notiz.

**Modell.** Der Stand der Überführung (wie viele berechnete Spalten und Zellen ohne Mappe gerechnet werden, der Rest
nach Grund), eine Übersicht aller Regionen, und je Region ihre Spalten mit Typ, Logik (änderbar), was sie lesen und
ihren Ausnahmezeilen. Bei einem Stapel steht, wo die Blätter in der Logik voneinander abweichen. »umbenennen« gibt
einer Region einen Namen, der sagt, was sie ist (erkannte Regionen heißen oft nach ihrem Blatt mit einer Nummer);
Daten, Logik und Verweise folgen. »Daten gegen die Mappe prüfen« und »Blatt hinzufügen« sitzen hier.

**Mappe.** Das Blatt, wie es in der Mappe liegt, mit Füllfarben, Schrift und verbundenen Zellen; die Blätter sind
Reiter am unteren Rand. Farbige Rahmen sind die erkannten Regionen (blau Tabelle, lila Einzelzellen, orange Block);
ein Klick auf den Namen führt in die Daten. Eine angeklickte Zelle steht mit ihrer Adresse im Namensfeld, Wert oder
Formel neben ƒx, und lässt sich dort überschreiben; danach wird die Mappe gerechnet und neu gelesen.

**Oben rechts.** »Mappe nicht aktuell« heißt: Daten oder Logik sind geändert, die Mappe ist noch auf dem alten
Stand. »Mappe rechnen« trägt beides ein und rechnet wie Excel. »Rückgängig« nimmt die letzte Änderung zurück (die
letzten 20 werden gehalten). »Zurück nach Excel« schreibt die Mappe als `<Name>.new.xlsx` in den Arbeitsordner und
lädt sie herunter; vorher wird gerechnet, falls nötig.

### Aufbau und Bauen der App

| Teil | Inhalt | Zeilen |
|---|---|---|
| `app/core` (reines Dart) | Schnittstelle (`Backend`: HTTP, Chat, eingebaut), Zustand und Abläufe (`App`), die Bildschirme als Bausteine (`Screen`, `Part`), Texte, Farben | 1.330 |
| `app/web` (Jaspr) | zeichnet die Bausteine als HTML (Aussehen in `web/app.css`); Seite, Dialoge, Dateiauswahl | 480 |
| `app/desktop` (Flutter) | zeichnet die Bausteine als Widgets; Fenster, Dialoge, Ordnerwahl, Brücke zur Engine | 910 |

Ein Baustein sagt, was da ist und was es tut (`Act`: etwas zum Drücken, `Entry`: eine Zeile zum Tippen, `DataGrid`:
die Zeilen einer Tabelle …), nicht wie es aussieht. Eine neue Funktion der App wird im Kern gebaut und erscheint in
allen Oberflächen; eine Oberfläche ändert man nur, um anders zu zeichnen.

    tools/build_app.sh                                       # baut app/web und legt sie in crates/xl-mcp/web
    (cd app/core && dart test)                               # Ablauftests des Kerns gegen den echten Server
    (cd app/web && dart test -p chrome)                      # die Seite im Chrome ohne Bildschirm, echter Server
    (cd app/desktop && flutter test integration_test -d macos)   # Ablauftest im echten Fenster, Engine eingebaut
    (cd app/desktop && flutter build macos --release)        # die Desktop-App, nur für Apple Silicon (rund 130 MB)

Die Tests des Kerns und der Seite brauchen `target/quick/xl-mcp` (`cargo build --profile quick -p xl-mcp`). Die des
Kerns bedienen die App ohne Oberfläche: Sie drücken, tippen und lesen die Bausteine, die eine Oberfläche zeichnen
würde. Die der Seite zeichnen sie mit Jaspr in Chrome ohne Bildschirm und bedienen das HTML wie ein Mensch (Klick,
Doppelklick, Tippen, Enter, Escape); der Server läuft daneben im Dart-VM. Zellen tragen `data-at="Zeile,Spalte"`,
damit Tests und Agenten sie finden. `XL_SHOTS=<Ordner>` vor dem Desktop-Test legt Bilder des Fensters ab. Für die
Arbeit an der Web-Oberfläche: `xl-mcp --app <Ordner> --web-dir app/web/build/jaspr`.

## 5. Begriffe

### Region

| Art | Was | Als DataFrame |
|---|---|---|
| `table` | Spalten unter einer Kopfzeile: eine Excel-Tabelle oder ein erkannter Block | eine Spalte je Spalte |
| `cells` | Einzelzellen, benannt nach definiertem Namen, sonst nach der Beschriftung links, sonst oben | im Speicher eine Zeile je Zelle |
| `block` | Zahlengitter ohne Kopfzeile | eine Spalte je Spalte |

Regionen werden Blatt für Blatt gefunden. Eine Region liegt in genau einem Blatt; ein Blatt kann mehrere haben.
Namen kommen von der Excel-Tabelle, einer Überschrift oder dem Blatt; gleiche Namen werden nummeriert.

Eine Kopfzeile darf zwei Zeilen haben: oben die Gruppe, darunter die Spalte. Der Spaltenname ist beides zusammen
(`MW VOLUME` über `Per EnPower` wird `MW VOLUME Per EnPower`); eine verbundene Zelle oben gilt für alle Spalten, über
die sie reicht. Die zweite Zeile zählt als Kopf, wenn sie nur Beschriftungen enthält und unter mindestens zwei davon
Zahlen oder Formeln stehen und kein Text.

### Stapel: gleich gebaute Blätter als eine Tabelle

Steht dieselbe Tabelle in mehreren Blättern (ein Blatt je Monat, je Standort), bleibt jedes Blatt eine Region. Dazu
gibt es einen *Stapel*: alle diese Tabellen untereinander, mit der Spalte `sheet` vorn.

```python
store.stacks                       # {"stack": {"columns": [...], "members": [{"sheet": "Jan", "region": "Jan"}, …]}}
alle = store.read("stack")         # sheet, item, qty, … für alle Blätter
store.write("stack", feb)          # Zeilen mit sheet == "Feb" gehen in das Blatt Feb
store.add_sheet("Mar", "Apr")      # neues Blatt als Kopie von Mar
store.set_logic("stack", "amount", "[@qty]*[@price]")   # in allen Blättern
```

- **Wann:** Tabellen in verschiedenen Blättern an derselben Stelle, mit denselben Spaltennamen und derselben Art von
  Daten je Spalte, mindestens zwei Spalten, mindestens zwei Blätter.
- **Logik darf abweichen:** Jedes Blatt behält seine Formeln. `store.stacks[name]["differs"]` und `describe` nennen
  die Spalten, deren Formel nicht in allen Blättern gleich ist.
- **Schreiben:** Die Zeilen eines Blatts ersetzen dessen Tabelle. Ein Blatt, das in den Zeilen nicht vorkommt, bleibt,
  wie es ist. Ein unbekannter Blattname wird abgelehnt; neue Blätter legt `add_sheet` an.
- **Name:** der gemeinsame Anfang der Regionsnamen (`Enron Purchases_2`, `Enron sale_2` → `Enron`), sonst `stack`.
- **Dateien:** Ein Stapel hat keine eigene Parquet-Datei; er ist die Sicht auf die Dateien seiner Blätter. Mit Polars
  allein: `pl.concat` über `data/Jan.parquet`, `data/Feb.parquet`, ….

### Logik

Die Formel einer berechneten Spalte oder Zelle, so geschrieben, dass sie ohne Zelladressen auskommt:

| Schreibweise | Bedeutet |
|---|---|
| `[@qty]` | Spalte `qty` derselben Zeile |
| `Sales[amount]` | die ganze Spalte `amount` der Tabelle `Sales` |
| `Tax_rate` | die Einzelzelle `Tax rate` (Leerzeichen und Zeichen werden `_`) |
| `Model!$B$1` | irgendeine Zelle nach Adresse |

Alle Funktionen, die der Rechenkern kennt, sind erlaubt; `set_logic` über MCP prüft die Formel sofort.

### Ausnahmezeilen

Eine Spalte ist *eine Formel*, wenn sie in mindestens zwei Zeilen und in zwei Dritteln der Zeilen steht. Die übrigen
Zeilen sind Ausnahmen: eine eigene Formel oder ein fester Wert. Umgekehrt ist eine Spalte *Werte*, wenn zwei Drittel
Werte sind; ihre Formelzeilen sind Ausnahmen. Ausnahmen bleiben, was sie sind:

- Eine Zeile mit eigener Formel behält sie und wird mit ihr gerechnet.
- Ein fester Wert in einer berechneten Spalte ist Daten: Er steht im DataFrame und kann dort geändert werden.
- Neue Zeilen bekommen die Formel der Spalte.

Ausnahmen hängen an der Zeilennummer, nicht an einem Schlüssel. Das gilt auch für feste Werte: Wer eine Tabelle ohne
die berechnete Spalte neu schreibt, behält den festen Wert in seiner Zeile, mit `recalculate` wie mit `run`.

### Notiz

Jede Parquet-Datei trägt in ihren Datei-Metadaten (Schlüssel `xlcells`, JSON) die Beschreibung ihrer Region: Blatt,
Bereich, je Spalte Typ, Formel, Logik, Eingaben und den Plan, nach dem gerechnet wird. Jedes Parquet-Werkzeug liest
sie. Wer die Datei mit einem Werkzeug neu schreibt, das Metadaten verwirft, muss die Notiz mitgeben
(`metadata={xo.NOTE_KEY: json.dumps(note)}`); im Speicher liegt sie zusätzlich in `notes.json`.

### Zurück nach Excel: die Mappe bleibt gebaut, wie sie war

Ändert sich der Inhalt einer Tabelle (Werte, mehr oder weniger Zeilen), stellt `run` den Aufbau der Mappe wieder her:

- Bekommt eine Tabelle Zeilen dazu, wandert alles, was unter ihr liegt, nach unten: die Summenzeile, Notizen, weitere
  Tabellen. Es werden ganze Blattzeilen eingefügt, wie in Excel mit »Zeilen einfügen«. Nimmt man Zeilen weg, wandert es
  wieder hoch.
- Bereiche, die an der letzten Zeile der Tabelle enden, folgen ihrer Länge: die Summe darunter (`SUM(D3:D5)` wird
  `SUM(D3:D7)`), Bezüge aus anderen Blättern, Filter, bedingte Formate, Gültigkeitsregeln, der Bereich einer
  Excel-Tabelle. Ein Bereich über einen Teil der Zeilen (`SUM(D3:D4)`) bleibt.
- Bezüge auf einzelne Zellen unterhalb wandern mit (`=Report!D6` wird `=Report!D8`). Ein Bezug auf eine Zeile, die es
  nicht mehr gibt, wird `#REF!`, wie in Excel.
- Liegt neben der Tabelle eine zweite in denselben Zeilen und läuft dort etwas über ihr Ende hinweg (die Summen beider
  in einer Zeile), wandern nur die Spalten der Tabelle. Würde das eine Zeile darunter zerreißen, werden beim Wachsen
  ganze Zeilen eingefügt; beim Schrumpfen bleiben die geleerten Zeilen stehen.
- Die Regionen des Speichers bleiben, was sie sind: Eine Tabelle bleibt Tabelle, auch mit einer einzigen Zeile, und
  jede Region behält ihren Namen, auch wenn sie verschoben wurde.
- Würden durch die neuen Zeilen Regionen verschwinden (eine Tabelle wächst in das hinein, was neben oder unter ihr
  liegt, und wird mit ihm als eins gelesen), lehnt `run` ab und lässt den Speicher, wie er ist.

Mehr Zeilen und wieder zurück ergibt in der Messung fast immer die Mappe, die es vorher war, Zelle für Zelle (Zahlen in
Abschnitt 10).

### Daten ohne Mappe

Ein Ordner mit Parquet-Dateien, die ihre Notizen tragen (zum Beispiel `data/` eines Speichers, woandershin kopiert),
ist für sich benutzbar: Tabellen beschreiben, lesen, Zeilen ändern, neu rechnen, als CSV oder Parquet ausgeben. Was die
Mappe braucht (Logik ändern, `run`, Blätter ansehen, zurück nach Excel), sagt das und tut nichts.

### Rückgängig

Jede Änderung über die App, über die MCP-Werkzeuge oder über `store.patch` und `store.set_cells` hält vorher den
Stand des Speichers fest (im Ordner `.undo`, die letzten 20). `undo` stellt ihn wieder her. Eine Änderung, die
scheitert, lässt den Speicher, wie er war.

### `recalculate` und `run`

| | `store.recalculate()` | `store.run()` |
|---|---|---|
| Braucht die Mappe | nein | ja |
| Rechnet | was einen Plan hat (gemessen 83,7 % der berechneten Zellen) | jede Formel, die der Rechenkern kann |
| Der Rest | behält seinen Wert, mit Grund in `kept` | steht mit Grund in `not_computed` |
| Geänderte Logik (`set_logic`) | wirkt nicht | wirkt |
| Schreibt | die Parquet-Dateien | Parquet, `model.json`, `workbook.xlsx` |

`recalculate` ist für die Anwendung, die ohne Mappe läuft. `run` ist die Abstimmung mit der Mappe.

## 6. Python-Schnittstelle

Alles hängt an der openpyxl-Fassade `xlcells.openpyxl` (`xo`). DataFrames sind Polars oder pyarrow.

### Mappe

| Aufruf | Ergebnis |
|---|---|
| `xo.load_workbook(pfad)` | Mappe (`.xlsx`, `.xlsm`, `.xls`) |
| `wb.regions`, `ws.regions` | Regionen nach Namen (der Mappe, eines Blatts) |
| `wb.calculate()` | rechnet die Formeln der Mappe |
| `wb.evaluate({"Sales": df})` | Regionen als DataFrames, nachdem `df` eingesetzt und alles gerechnet ist; die Mappe bleibt unverändert; `.not_computed` nennt, was nicht ging |
| `wb.to_polars_code(pfad, accuracy="auto")` | die Mappe als Python-Programm mit Polars; `accuracy` ist `auto`, `plain` oder `exact`; `.summary()` sagt je Spalte und Zelle, wie sie geschrieben ist (`polars`, `plan`, `engine`) |
| `wb.to_store(pfad)` | legt einen Speicher an |
| `ws.to_polars(bereich=None, header=True)`, `ws.to_arrow(…)` | ein Blatt oder ein Bereich als DataFrame |
| `zelle.look` | was die Zelle zeigt (z. B. `look.fill_color`, `look.num_format`), mit Herkunft |

### Region (`wb.regions["Sales"]`)

| Aufruf | Ergebnis |
|---|---|
| `.name`, `.kind`, `.sheet`, `.ref`, `.source` | Name, Art, Blatt, Bereich, `excel table` oder `detected` |
| `.columns` | Spalten: `.name`, `.letter`, `.dtype`, `.formula`, `.logic`, `.calculated`, `.exceptions` |
| `.cells` | Einzelzellen: `.name`, `.coordinate`, `.dtype`, `.formula`, `.logic`, `.calculated` |
| `.to_polars()`, `.to_arrow()` | die Daten (Einzelzellen hier als eine Zeile mit einer Spalte je Zelle) |
| `.write(df)` | setzt `df` in die Zellen; Tabellen wachsen und schrumpfen, Formeln ziehen mit; danach `wb.calculate()` |
| `.verify(df)` | die Zellen, in denen `df` von der Mappe abweicht (`region, cell, column, row, expected, actual`) |

Eine Ausnahme (`column.exceptions`): `.row` (ab 0 in den Daten), `.coordinate`, `.formula`, `.logic` (`None` bei
einem festen Wert).

### Speicher (`xo.open_store(pfad)` oder `wb.to_store(pfad)`)

| Aufruf | Ergebnis |
|---|---|
| `print(store)` | Regionen, Spalten, Typen, Logik |
| `store.regions`, `store.model` | das Modell als dict |
| `store.stacks` | die Stapel nach Namen (`columns`, `members`, `differs`) |
| `store.add_sheet(wie, name)` | neues Blatt als Kopie des Blatts `wie`; gibt die Regionen des neuen Blatts |
| `store.read(region)` | DataFrame; Einzelzellen als Zeilen `name, cell, dtype, number, text`; ein Stapel mit `sheet` vorn |
| `store.values(region)` | Einzelzellen als dict |
| `store.write(region, df)` | ersetzt die Daten. Tabelle: alle Zeilen, berechnete Spalten dürfen fehlen. Einzelzellen: nur die genannten |
| `store.set_values(region, {"Tax rate": 0.07})` | setzt Einzelzellen |
| `store.patch(region, set=, delete=, insert=, at=)` | ändert einzelne Zeilen: `set` `[{"row", "column", "value"}]` (Einzelzellen `{"name", "value"}`), `delete` `[zeile, …]`, `insert` `[{…}]` vor Zeile `at` oder am Ende |
| `store.undo()` | nimmt die letzte festgehaltene Änderung zurück; gibt, was es war, oder `None` |
| `store.rename(region, name)` | gibt einer Region einen anderen Namen; Daten und Logik folgen, die Mappe selbst ändert sich nicht |
| `store.set_cells(blatt, {"B7": 5, "D4": "=B4*2"})` | trägt in Zellen der Mappe ein und rechnet; gibt den Bericht von `run` |
| `store.tables` | die Tabellen nach ihren Notizen: je Spalte `dtype`, `logic`, `reads`, `state` (`calculated` oder `kept` mit `why`), `exceptions` |
| `store.set_logic(region, name, logik)` | neue Logik; `None` macht daraus Daten; wirkt mit `run` |
| `store.recalculate()` | `{"done", "kept", "from_kept"}` |
| `store.run()` | `{"formula_cells", "computed", "not_computed", "regions", "logic_changed"}` |
| `store.notes[region]` | die Notiz der Region |
| `store.workbook()` | die Mappe des Speichers |
| `store.to_port(ordner, accuracy="auto", replace=False)` | die Mappe als Python-Projekt auf Polars (Abschnitt 8); gibt `summary` und je berechneter Spalte und Zelle `how` und `line` |

Im Bericht von `recalculate`:

- `done`: `(Region, Name)` neu gerechnet. Eine Ausnahmezeile heißt `Spalte (row n)`.
- `kept`: `(Region, Name, Formel, Grund)` hat den alten Wert behalten.
- `from_kept`: `(Region, Name, woraus)` ist neu gerechnet, liest aber etwas Behaltenes. Der Wert stimmt nur, wenn
  sich das Behaltene nicht geändert hätte.

### Dateien ohne Speicher

`xo.open_store(ordner)` nimmt auch einen Ordner mit Parquet-Dateien und Notizen ohne Mappe: `read`, `write`, `patch`,
`recalculate`, `tables` und `undo` gehen dort.

| Aufruf | Ergebnis |
|---|---|
| `xo.read_note(pfad)` | die Notiz einer Parquet-Datei, oder `None` |
| `xo.recalculate([pfad, …])` | rechnet die Dateien aus ihren Notizen neu; alle Dateien angeben, die gelesen werden |
| `xo.NOTE_KEY` | `"xlcells"` |

## 7. MCP-Server und Schnittstelle der App (`xl-mcp`)

Für Agenten. Zwei Fassungen desselben Servers: das Programm `xl-mcp` (ohne Python, für einen Rechner gebaut) und
`xlcells-mcp` aus dem Python-Paket (der Server als WebAssembly-Modul, für jeden Prozessor; dieselben Argumente).

    xl-mcp [--log <Datei>]                                        # stdin/stdout, Agent auf demselben Rechner
    xl-mcp --app <Ordner> [--port 8787] [--log <Datei>]           # die App im Browser, nur für den eigenen Rechner
    xl-mcp --http 127.0.0.1:8787 --workspace <Ordner> --token <Geheimnis> [--public-url https://…] [--log <Datei>]

Mit `--log` schreibt der Server jeden Werkzeugaufruf als JSON-Zeile in die Datei: Werkzeug, Argumente, Antwort, ob
Fehler, Dauer in Millisekunden.

Im HTTP-Betrieb: `POST /mcp/<Geheimnis>` für die Werkzeuge, `GET /files/<Geheimnis>` als Seite zum Hoch- und
Herunterladen. Mappen liegen im Arbeitsordner und werden beim Namen genannt; Pfade nach außen werden abgelehnt.

| Werkzeug | Argumente | Tut |
|---|---|---|
| `list_workbooks` | – | Mappen, die geöffnet werden können (nur im HTTP-Betrieb, wo es einen Arbeitsordner gibt) |
| `open_workbook` | `path` | legt den Speicher `<Datei>.store` an, nennt die Regionen |
| `describe` | `store`, [`region`, `offset`, `limit`] | Übersicht, oder Spalten/Zellen einer Region mit Typ, Logik, Ausnahmen |
| `read_region` | `store`, `region`, [`offset`, `limit`, `columns`, `filter`, `search`, `sort`] | Zeilen als JSON; Einzelzellen als `{name, cell, value}`; mit Filter oder Sortierung nennt `index` die Zeilennummern |
| `change_rows` | `store`, `region`, [`set`, `delete`, `insert`, `then`] | ändert einzelne Zeilen und rechnet danach aus den Daten allein neu (`then`: `recalculate`, `run`, `nothing`) |
| `write_region` | `store`, `region`, `rows` | ersetzt die Daten |
| `append_rows` | `store`, `region`, `rows` | hängt Zeilen an eine Tabelle |
| `set_value` | `store`, `region`, `name`, `value` | setzt eine Einzelzelle |
| `set_logic` | `store`, `region`, `name`, [`logic`] | setzt die Logik, prüft sie sofort |
| `add_sheet` | `store`, `like`, `name` | neues Blatt als Kopie eines vorhandenen |
| `rename_region` | `store`, `region`, `name` | gibt einer Region einen anderen Namen |
| `run` | `store` | wie `store.run()`, nennt Zellen ohne Wert mit Grund |
| `recalculate` | `store` | rechnet aus den Daten allein, ohne Mappe; nennt, was so nicht geht |
| `undo` | `store` | nimmt die letzte Änderung zurück |
| `read_range` | `store`, `sheet`, `range` | Zellen nach Adresse, höchstens 500 |
| `verify` | `store` | ob Daten und Mappe übereinstimmen |
| `export_workbook` | `store`, `path` | zurück nach Excel: schreibt die Mappe als `.xlsx`, gebaut wie das Original; rechnet vorher, falls nötig |
| `export_region` | `store`, `region`, `what`, [`path`, `filter`, `search`, `sort`] | eine Region als `csv` (auch ein Ausschnitt) oder `parquet` |
| `polars_code` | `store`, [`accuracy`] | das Polars-Programm (`plain` oder `exact`) |
| `export_port` | `store`, [`path`, `accuracy`, `replace`] | die Mappe als Python-Projekt (Abschnitt 8); im HTTP-Betrieb zusätzlich als Zip mit Link |
| `port_guide` | [`topic`: `way`, `layout`, `excel-to-polars`, `hosts`] | die Anleitung für Agenten zur Portierung (`skills/excel-port`) |
| `show_app` | `store`, [`level`, `item`] | zeigt den Speicher als App im Gespräch (Chats, die MCP Apps zeigen; sonst nur Text) |
| `app_call` | `op`, [`args`] | die Aufrufe der App selbst (für Agenten nicht sichtbar, wo der Chat das beachtet) |

Beispiel, `describe` mit `region: "Sales"`:

    Sales (table Sales!A1:D5)
      item: String
      qty: Int64
      price: Int64
      amount: Float64  = ROUND([@qty]*[@price]*(1+Tax_rate),2)
          but D4: a plain value

Der Server bietet außerdem den Prompt `port_workbook` (Argumente `workbook`, `goal`): die Aufgabe, eine Mappe zu
portieren, mit der Anleitung.

Stapel erscheinen in `describe` unter den Regionen; `read_region`, `write_region`, `append_rows` und `set_logic` nehmen den
Namen eines Stapels wie den einer Region.

### Schnittstelle der App

`POST /api/<Geheimnis>/<Aufruf>` mit den Argumenten als JSON, Antwort JSON; bei einem Fehler Status 400 und
`{"error": …}`. Im Betrieb `--app` ist das Geheimnis `local`. `store` ist der Name des Speichers im Arbeitsordner
(`model.xlsx.store`) oder eines Ordners mit Parquet-Dateien.

| Aufruf | Argumente | Antwort |
|---|---|---|
| `workspace` | – | `files` (Excel-Dateien, je mit `store` oder `null`), `stores` (je `name`, `workbook`) |
| `open` | `file` | legt den Speicher an: `store` |
| `tables` | `store` | `tables` (wie `store.tables`), `stacks`, `workbook`, `pending`, `undo`, `views` |
| `rows` | `store`, `region`, [`view`, `offset`, `limit`, `columns`, `summary`] | `total`, `all`, `index`, `rows`, `summary` |
| `patch` | `store`, `region`, `patch`, [`then`] | `rows` und der Bericht dessen, was danach gerechnet hat |
| `recalculate`, `run` | `store` | der Bericht |
| `set_logic` | `store`, `region`, `name`, [`logic`] | prüft, setzt und rechnet |
| `add_sheet` | `store`, `like`, `name` | `regions` des neuen Blatts |
| `rename` | `store`, `region`, `name` | benennt eine Region um und rechnet |
| `undo` | `store` | `undone` |
| `verify` | `store` | `text` |
| `sheets` | `store` | je Blatt `name`, `rows`, `cols`, `regions` |
| `cells` | `store`, `sheet`, [`top`, `left`, `rows`, `cols`] | `cells` als `[Zeile, Spalte, Text, Formel, Stil]`, `styles`, `merges`, `widths` |
| `set_cells` | `store`, `sheet`, `cells` (`[{cell, value}]`) | trägt ein und rechnet |
| `save_view` | `store`, `name`, `region`, `view` (`null` löscht) | alle Ausschnitte |
| `export` | `store`, [`what`: `workbook`, `csv`, `parquet`; `region`, `view`, `path`] | `file`, `url` |

Ein Ausschnitt (`view`): `{"filter": [{"column", "op", "value"}], "search": "…", "sort": [{"column", "desc"}]}`.

## 8. Portierung: die Mappe als Python-Projekt

Eine Mappe wird ein Python-Programm auf Polars, das rechnet, was die Mappe rechnet, Wert für Wert gegen die Mappe
geprüft, und dessen Ergebnisse zurück in die Mappe können. xlcells schreibt die erste Fassung; ein Agent (oder ein
Mensch) bringt sie zu Ende und baut die Anwendung darum. Die Anleitung für Agenten liegt in `skills/excel-port`
(als Skill für Claude im Release, über MCP als `port_guide` und Prompt `port_workbook`).

    store.to_port("plan-port")                 # Python
    export_port {store}                        # MCP; im HTTP-Betrieb auch als Zip zum Herunterladen
    cd plan-port && uv run check.py            # Polars genügt

Das Projekt:

| Datei | Inhalt |
|---|---|
| `data/<Region>.parquet` | die Eingaben (ohne Berechnetes) |
| `expected/<Region>.parquet` | dasselbe mit allem, was die Mappe rechnet: der Maßstab |
| `logic.json` | Regionen, Eingaben, jede berechnete Spalte und Zelle mit Formel, Logik, Art, Grund, Zeile |
| `cells.json` | die Zellen der Blätter (für Formeln, die Zellen außerhalb der Regionen lesen) |
| `workbook.xlsx` | die Mappe: Rückweg, und der Rechenkern für den Rest |
| `port/calc.py` | das Programm: `calculate(ordner)` gibt jede Region als Frame |
| `port/xl.py` | was es neben Polars braucht |
| `check.py`, `run.py`, `tests/` | prüfen (`--exact --data ORDNER` gegen den Rechenkern auf anderen Daten), rechnen (`--excel DATEI` zurück in die Mappe), Tests |

Jede berechnete Spalte und Zelle ist portiert als `polars` (lesbare Polars-Ausdrücke), `plan` (die Übersetzung des
Rechenkerns, als Polars-Code ausgeschrieben: genau, aber nicht zum Lesen) oder `rest` (noch nicht: mit xlcells
rechnet sein Rechenkern, ohne gelten die Werte aus `expected/`, und `check.py` meldet sie als nicht berechnet). Das
Projekt läuft also von Anfang an mit Polars allein; der Rest ist benannt.

Mit `accuracy="auto"` (Vorgabe, wo Python läuft: im Paket und in `xlcells-mcp`) prüft sich das Projekt beim Schreiben:
`check.py` läuft, eine lesbare Form mit anderen Werten als die Mappe wird zur Übersetzung des Rechenkerns, eine
Übersetzung, die auch nicht stimmt, wird Rest. Das Programm `xl-mcp` (ohne Python) schreibt ungeprüft. Zellen, deren
Wert vom Zeitpunkt abhängt (`TODAY`, `NOW`, `RAND`), werden nicht verglichen.

Geprüft: `bindings/python/tests/test_port.py` (Projekt erzeugen und prüfen, mit Polars allein, falscher Code wird mit
Zeile gefunden, andere Daten gegen den Rechenkern, Rückweg in die Mappe, Stapel, ein Projekt wird nicht
überschrieben). Enron-Mappe `enron_purchases.xls`: 26 von 32 berechneten Spalten und Zellen eigenständig (11
lesbar, 15 Übersetzung), alle richtig, 6 Rest (`NOW`, eine Datumsreihe, Teilsummen).

Grenzen: Die Namen der Regionen sind die gefundenen; mit `rename_region` vergebene Namen übernimmt das Projekt
nicht. Die Übersetzung schreibt Ausnahmezeilen mit eigener Formel noch nicht aus (sie bleiben Rest).

## 9. Rust

| Krate | Inhalt |
|---|---|
| `xl-cells` | Mappe ↔ spaltenförmige Zelltabelle (`Workbook`) |
| `xl-engine` | Rechenkern ohne Polars: `regions::regions(&wb)` und `regions_like` (bekannte Regionen bleiben), `apply::write_region` / `verify` / `set_cells`, `rows` (Zeilen eines Blatts verschieben), `view` (Zeilen als JSON, Ausschnitte, geänderte Zeilen), `codegen::polars_program`, `store` (auch `patch`, `remember`, `undo`, `tables`, `set_cells`), `plans::recalculate`, `long` |
| `xl-polars` | Ausführung auf Polars; `xl_polars::store::{create, read_region, read_wide, write_frame, run, recalculate}` nehmen ein Verzeichnis |
| `xl-mcp` | der Server: `tools.rs` (Werkzeuge der Agenten), `api.rs` (Schnittstelle der App), `http.rs`, `web/` (die App) |
| `xl-look` | Aussehen einer Zelle |

`store::run` gibt `RunReport` (`formula_cells`, `computed`, `not_computed`, `regions`, `logic_changed`,
`uncomputed`), `store::recalculate` gibt `plans::Recalculation` (`done`, `kept`, `from_kept`).

## 10. Grenzen

**Rechnen ohne Mappe**

- 16,3 % der berechneten Zellen behalten bei `recalculate` ihren Wert. Größter Posten: Nachschlagen (`MATCH`,
  `VLOOKUP`, `OFFSET`, `COUNTIF`/`SUMIF`), dessen Tabelle fest im Plan steht statt als Eingabe.
- Add-in-Funktionen, DDE-Verknüpfungen und flüchtige Funktionen (`TODAY`, `RAND`) rechnet auch `run` nicht.
- Bereiche über mehrere Blätter (`SUM(Jan:Dez!B2)`) haben keinen Plan.

**Ausnahmezeilen**

- Sie hängen an der Zeilennummer. Im Beispiel oben liegt die 99 nach dem Schreiben neuer Daten auf Zeile `c`, weil `c`
  jetzt die dritte Zeile ist. Wer sortiert oder Zeilen einfügt, verschiebt die Daten unter den Ausnahmen hindurch.

**Blätter**

- Stapel gibt es nur für Tabellen. Einzelzellen, die in jedem Blatt gleich liegen (eine Summe unter jeder
  Monatstabelle), bleiben einzelne Regionen.
- Gestapelt wird nur, was an derselben Stelle liegt und dieselben Spaltennamen hat. `TOTAL COST` in einem Blatt und
  `TOTAL REVENUE` im anderen sind zwei Tabellen.
- `add_sheet` kopiert Zellen, Formate, verbundene Zellen, Breiten, Gültigkeitsregeln und bedingte Formate. Excel-Tabellen,
  Bilder und Diagramme des Blatts kopiert es nicht.
- Blätter anlegen, löschen, umbenennen geht über die openpyxl-Fassade, nicht über den Speicher oder MCP.

**Zurück nach Excel**

- Wiederhergestellt wird der Aufbau bei geändertem Inhalt. Neue Spalten oder neue Tabellen in den Daten gehen nicht in
  die Mappe; Spalten lassen sich weder anlegen noch löschen.
- Wo zwei Tabellen nebeneinander verschieden lang werden oder eine Tabelle beim Wachsen an ihre Nachbarn stößt, gibt es
  keinen Aufbau, der alles erhält. `run` lehnt dann ab, wenn Regionen verschwänden; sonst bleibt eine Lücke oder die
  Summenzeilen stehen nicht mehr in einer Zeile.
- Ein Bezug auf eine einzelne gelöschte Zeile wird `#REF!` und kommt nicht wieder, wenn die Zeile zurückkommt.
- Eine `.xls`-Mappe kommt als `.xlsx` zurück (der Speicher hält sie von Anfang an so).
- Formeln in Gültigkeitsregeln und bedingten Formaten wandern beim Einfügen von Zeilen nicht mit (ihre Bereiche schon).
  Bezüge auf ganze Zeilen (`5:7`) auch nicht.

**Web-App**

- Nur lokal, ein Benutzer, ohne Anmeldung. Das Blatt zeigt höchstens 100 Spalten (Zeilen werden nachgeladen), Zahlenformate nur
  grob (Datum, Prozent, Nachkommastellen, Tausender), keine Rahmen, Diagramme oder Bilder.
- Bei einem Stapel sind Ausnahmezeilen nur für das erste Blatt markiert.
- »Rückgängig« kopiert den ganzen Speicher je Schritt; bei sehr großen Mappen kostet das Zeit und Platz.
- Logik lässt sich nur ändern, wo die Mappe da ist: Aus einer neuen Formel macht die Mappe den Plan.

**Erkennung**

- Regionsnamen wie `1_2` und breite Einzelzellen-Regionen, die eigentlich Tabellen sind, kommen in echten Mappen vor
  und sind nicht gelöst. Kopfzeilen mit mehr als zwei Zeilen werden nicht erkannt.
- Eine erkannte Region lässt sich umbenennen, aber nicht teilen, zusammenlegen oder in ihrer Art ändern. Einzelzellen
  lassen sich nicht umbenennen.

**Geprüft und nicht geprüft**

- Geprüft: an 501 Mappen (Enron, LibreOffice-Testmappen) gegen die Rechnung der Mappe; Rückschreiben ohne geänderte
  Zelle; ein Agentenlauf über MCP an zwei Mappen; Claude Desktop über MCP mit drei Aufgaben an zwei Mappen (alle
  richtig gelöst, vor Kopfzeilen und Stapeln).
- Rückweg bei geänderter Zeilenzahl, an denselben 501 Mappen (`shape_check`, bis zu drei Tabellen je Mappe, 743
  Tabellen): zwei Zeilen mehr gehen bei 729 mit unverändertem Aufbau, 14 lehnt `run` ab; von den 729 kommen 728 nach dem
  Wegnehmen der Zeilen Zelle für Zelle zur alten Mappe zurück; eine Zeile weniger geht bei 706 von 729, 23 lehnt `run` ab.
- In Excel geprüft (`tools/excel_shape.py`, eine gebaute Mappe mit Excel-Tabelle, Summe, bedingtem Format,
  Gültigkeitsregel, definiertem Namen): Die um drei Zeilen gewachsene Mappe öffnet ohne Reparatur, Excel rechnet
  dieselben Werte, Tabelle, Format, Regel und Name liegen richtig.
- Die Seite in Chrome ohne Bildschirm (`app/web/test`, 8 Abläufe: überführen, Wert per Doppelklick ändern, Escape,
  filtern, Rückgängig, Mappe mit Namensfeld und Blattreitern, Umbenennen im Dialog, Frage ablehnen). Der erste Lauf
  fand einen Fehler: ein Feld meldete sich nach Enter oder Escape beim Verlassen ein zweites Mal fertig und schickte
  den alten Wert.
- Die App im Browser (Jaspr-Fassung, Chrome) an zwei Mappen bedient: überführen, Wert ändern, filtern, Logik in der
  Formelzeile, Modell, Mappe mit Farben, Stapel, Ausschnitt merken (eigener Dialog). Die Fassung davor (`app.js`) war
  zusätzlich mit Zeile anfügen und löschen, sortieren, Rückgängig, Mappe rechnen und Zelle überschreiben bedient;
  diese Abläufe prüfen jetzt die Tests des Kerns (`app/core/test`, 12 Abläufe gegen den echten Server, darunter Stapel mit neuem Blatt und Daten ohne Mappe). Im Browser
  nicht bedient: Hochladen, Herunterladen, Blatt hinzufügen; Safari und Firefox.
- Die App im Chat: auf der Prüfseite (`chat.html`, abgeschotteter Rahmen, Regeln nach der Spezifikation der MCP
  Apps vom 2026-01-26) bedient (Wert ändern, Modell, Umbenennen mit Dialog), und der Weg über den Host als Test des
  Kerns. Claude Desktop meldet beim Start die Erweiterung für MCP Apps (`io.modelcontextprotocol/ui` mit
  `text/html;profile=mcp-app`), im Chat wie im Agentenmodus; ob es die App zeigt, ist noch nicht gesehen.
- Die Desktop-App: auf macOS durch drei Ablauftests im echten Fenster mit eingebauter Engine (eine gebaute Mappe:
  überführen, Wert ändern, Logik, Modell, Mappe, zurück nach Excel; gleich gebaute Blätter mit verbundener Zelle;
  eine Enron-Mappe: Stapel, Blatt mit Farben, Modell). Die Release-Fassung (Apple Silicon) mit Maus und Tastatur
  bedient: überführen, Wert per Doppelklick ändern, Escape, Mappe rechnen, Zelle über die Bearbeitungsleiste ändern,
  Blattreiter, Umbenennen im Dialog, zurück nach Excel (Datei mit openpyxl geprüft). Dabei gefunden und behoben: Feld
  und Dialog markierten ihren Text nicht, das Getippte kam hinter den alten Wert. Ordnerwahl und Dateiauswahl nicht
  geprüft. Unter Windows weder gebaut noch geprüft.
- Claude Code (ohne Bildschirm, nur die Werkzeuge des Servers, eine Aufgabe auf Deutsch: überführen, beschreiben,
  Steuersatz und eine Menge ändern, rechnen, prüfen, zurück nach Excel, als App zeigen): alles richtig, 13 Aufrufe,
  die abweichende Zelle und `TODAY()` von sich aus genannt; die geschriebene Mappe mit openpyxl geprüft.
- Die neuen Werkzeuge mit einem Agenten, der nur die Auskünfte des Servers kannte (eine Mappe, sechs Aufgaben:
  Zeilen ändern, gefiltert lesen, umbenennen und zurücknehmen, ohne Mappe rechnen, zurück nach Excel): alle Ergebnisse
  richtig, ein vergeblicher Aufruf (`list_workbooks` ohne Arbeitsordner). Seine vierzehn Anmerkungen zu unklaren
  Auskünften sind in die Beschreibungen eingearbeitet; ein zweiter Lauf danach steht aus.
- Nicht geprüft: mit Copilot Studio; mit ChatGPT; Stapel mit einem Agenten; `export_region` mit einem Agenten; echte
  Mappen mit Excel-Tabellen in Excel; Claude Desktop mit dem neuen Stand.
