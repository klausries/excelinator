# Ein Copilot-Studio-Agent, der Mappen portiert (nicht geprüft)

Microsoft Copilot lädt keine Python-Pakete; es spricht mit excelinator über den MCP-Server im HTTP-Betrieb. Diese
Anleitung ist aus der Dokumentation zusammengestellt und noch nie in einem Tenant ausprobiert worden. Was dort
anders ist, gehört hierher.

## 1. Den Server erreichbar machen

Auf einem Rechner mit dem Release (Windows: der entpackte Ordner; sonst `uv tool install xlcells-….whl`):

    xlcells-mcp --http 127.0.0.1:8787 --workspace C:\excelinator\arbeit --token <mindestens 16 Zeichen> --public-url https://<adresse>

Copilot Studio erreicht nur HTTPS-Adressen im Internet. Zum Ausprobieren gibt `cloudflared tunnel --url
http://localhost:8787` eine solche Adresse (ohne Konto); sie kommt als `--public-url` in den Aufruf oben. Der Server
ist dann nur durch das Geheimnis in der Adresse geschützt: nur während der Tests laufen lassen, nur auf einem
Arbeitsordner ohne andere Daten.

## 2. Der Agent

Copilot Studio → neuer Agent → Werkzeuge → Model Context Protocol, Adresse `https://<adresse>/mcp/<Geheimnis>`,
generative Orchestrierung an. Als Anweisungen des Agenten:

```text
You port Excel workbooks to Python programs on Polars with the excelinator tools, and you
work on the data and logic of workbooks for the person.

Before porting, call port_guide and follow it. Workbooks come in through the files page
of the server (the person uploads them; list_workbooks names them). export_port makes the
project and gives a link to its zip. Where you can run Python (code interpreter), unpack
it there and run check.py; otherwise give the person the link and tell them how to run
it (uv run check.py). Report only what check.py shows. Never change expected/.
```

Hat der Agent einen Code-Interpreter, kann er das Projekt selbst prüfen und die Reste portieren
(`reference/hosts.md` der Anleitung). Ob er Dateien von der Adresse des Servers holen darf, ist nicht geprüft;
sonst lädt die Person die Zip-Datei herunter und hängt sie im Gespräch an.

## 3. Ausprobieren

1. Eine Mappe über `https://<adresse>/files/<Geheimnis>` hochladen.
2. Dem Agenten sagen: »Portiere plan.xlsx in ein Python-Projekt und zeig, dass es stimmt.«
3. Im Protokoll (`--log <Datei>` beim Start) nachsehen, welche Werkzeuge er rief und wo er stolperte.
