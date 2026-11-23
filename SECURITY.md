# Security

Please report a vulnerability privately through GitHub:
**Security → Report a vulnerability** on this repository. Do not open a public issue.

Worth knowing:

- `xlcells-mcp --http` (and `xl-mcp --http`) is protected only by the secret in its
  address. It is meant for tests behind a tunnel, on a folder that holds nothing else.
- `--app` answers only pages of the same machine (it checks Host and Origin).
- Workbooks are untrusted input: excelinator never runs macros, but a crash or a hang on a
  crafted file is a bug worth reporting.

This is pre-alpha software: there are no supported versions yet beyond the latest release.
