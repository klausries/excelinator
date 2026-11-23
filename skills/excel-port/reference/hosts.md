# Where the port is made and run

| Where | MCP server | Python for the project |
|---|---|---|
| Claude Code, or a laptop with uv | `xlcells-mcp` (stdio), files by path | `uv run check.py` in the project; `uv run --with xlcells …` for the engine |
| Claude Desktop | `xlcells-mcp` as a local server (stdio); Windows: the folder of the release, `python\python.exe -m xlcells.mcp` | Claude's code tool, or the person's own Python |
| Claude or ChatGPT in the browser with code execution | `xlcells-mcp --http … --token …` reachable from the internet | the sandbox: fetch the zip of `export_port` (its download link), unpack, `pip install polars` (and `xlcells` for `--exact`, from PyPI or the wheel of the release) |
| Microsoft Copilot Studio | `xlcells-mcp --http …` as an MCP tool of the agent | Copilot's code interpreter, if the agent has one; otherwise the person runs the project |

Not checked yet: whether a chat's sandbox may fetch from the server (some only take
files attached to the conversation: then the person downloads the zip from the files
page and attaches it), and Copilot's code interpreter with such files.
