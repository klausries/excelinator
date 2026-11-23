"""Builds a release of excelinator: the Python package and the Windows folders.

    uv run python tools/build_release.py [--source URL] [--skip-wasm] [--only wheel|windows]

from the root of the repository. It writes to dist/:

* xlcells-VERSION-py3-none-any.whl: the Python package with its engine as one
  WebAssembly module (built here by tools/build_wasm.sh), for every processor.
* excelinator-VERSION-windows-x64.zip and -windows-arm64.zip: folders that run as they
  are unpacked, nothing to build or install: the embeddable Python of python.org, the
  package with Polars and wasmtime unpacked beside it, the same as wheels, uv, and .cmd
  files to start the MCP server, the app and Python.
* excel-port-skill.zip: the guide for agents (skills/excel-port) as a skill to upload
  into Claude.
* SHA256SUMS: the checksums of the above.

What it takes from elsewhere is pinned and checked: wheels from PyPI by their SHA-256,
Python from python.org, uv from its GitHub release (downloads are kept in
target/downloads). The version is that of packaging/VERSION.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
WORK = ROOT / "target" / "release-build"
DOWNLOADS = ROOT / "target" / "downloads"

PYTHON = "3.13.17"
UV = "0.13.9"
# What the folders hold beside xlcells: (package, version), as the tests ran with them.
WHEELS = [("polars", "2.1.0"), ("polars-runtime-32", "2.1.0"), ("wasmtime", "51.0.0")]
ARCHES = {
    # name: (wheel platform tag, python.org suffix, uv target)
    "x64": ("win_amd64", "amd64", "x86_64-pc-windows-msvc"),
    "arm64": ("win_arm64", "arm64", "aarch64-pc-windows-msvc"),
}


def say(*text):
    print(*text, flush=True)


def fetch(url, name=None, sha256=None):
    """A file from the internet, kept in target/downloads; checked if a SHA-256 is given."""
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    path = DOWNLOADS / (name or url.rsplit("/", 1)[1])
    if not path.exists():
        say("  fetching", url)
        with urllib.request.urlopen(url) as r, open(path.with_suffix(".part"), "wb") as f:
            shutil.copyfileobj(r, f)
        path.with_suffix(".part").rename(path)
    if sha256 and hashlib.sha256(path.read_bytes()).hexdigest() != sha256:
        path.unlink()
        raise SystemExit("checksum of {0} does not match PyPI's; removed, try again".format(path.name))
    return path


def pypi_wheel(package, version, platform):
    """The wheel of a package for a platform (or the one for any), from PyPI."""
    with urllib.request.urlopen("https://pypi.org/pypi/{0}/{1}/json".format(package, version)) as r:
        files = json.load(r)["urls"]
    wheels = [f for f in files if f["filename"].endswith(".whl")]
    # The wheel built for the platform first; one for any platform only where there is none.
    for wanted in (platform, "any"):
        for f in wheels:
            if wanted in f["filename"][:-4].split("-")[-1].split("."):
                return fetch(f["url"], f["filename"], f["digests"]["sha256"])
    raise SystemExit("{0} {1} has no wheel for {2}".format(package, version, platform))


def version():
    return (ROOT / "packaging" / "VERSION").read_text().strip()


def build_wheel(source, skip_wasm):
    if not skip_wasm:
        say("building the WebAssembly module")
        subprocess.run(["sh", "tools/build_wasm.sh"], cwd=ROOT, check=True)
    wasm = ROOT / "bindings/python/python/xlcells/_wasm/xlcells.wasm"
    if not wasm.exists():
        raise SystemExit("no {0}: build it (tools/build_wasm.sh)".format(wasm))
    stage = WORK / "python"
    shutil.rmtree(stage, ignore_errors=True)
    shutil.copytree(
        ROOT / "bindings/python/python/xlcells",
        stage / "xlcells",
        ignore=shutil.ignore_patterns("__pycache__", "*.so", "*.pyd", "*.dylib"),
    )
    text = (ROOT / "packaging/python/pyproject.toml").read_text().replace("@VERSION@", version())
    if source:
        text = text.replace("@SOURCE@", source)
    else:
        text = text.replace('[project.urls]\nSource = "@SOURCE@"\n\n', "")
    (stage / "pyproject.toml").write_text(text)
    shutil.copy(ROOT / "packaging/python/README.md", stage / "README.md")
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy(ROOT / name, stage / name)
    DIST.mkdir(exist_ok=True)
    for old in DIST.glob("xlcells-*.whl"):
        old.unlink()
    say("building the wheel")
    subprocess.run(["uv", "build", "--wheel", "--out-dir", str(DIST), str(stage)], cwd=ROOT, check=True)
    wheel = next(DIST.glob("xlcells-{0}-py3-none-any.whl".format(version())))
    say("  ", wheel.name, round(wheel.stat().st_size / 1e6, 1), "MB")
    return wheel


def unpack_wheel(wheel, lib):
    """A wheel unpacked as an installer would put it into site-packages."""
    with zipfile.ZipFile(wheel) as z:
        for item in z.infolist():
            parts = item.filename.split("/")
            # name.data/purelib/… and …/platlib/… go to the top; scripts and headers are left out.
            if parts[0].endswith(".data"):
                if len(parts) < 3 or parts[1] not in ("purelib", "platlib"):
                    continue
                parts = parts[2:]
            target = lib.joinpath(*parts)
            if item.is_dir():
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(z.read(item))


def build_windows(wheel, arch):
    platform, py_suffix, uv_target = ARCHES[arch]
    name = "excelinator-{0}-windows-{1}".format(version(), arch)
    say("building", name)
    folder = WORK / name
    shutil.rmtree(folder, ignore_errors=True)
    (folder / "wheels").mkdir(parents=True)
    # xlcells as a wheel too (to install into an environment of one's own); what it
    # needs only unpacked, not twice.
    shutil.copy(wheel, folder / "wheels" / wheel.name)
    for w in [wheel] + [pypi_wheel(p, v, platform) for p, v in WHEELS]:
        unpack_wheel(w, folder / "lib")
    # Python: the embeddable distribution, its path told to look into lib too.
    py = fetch("https://www.python.org/ftp/python/{0}/python-{0}-embed-{1}.zip".format(PYTHON, py_suffix))
    with zipfile.ZipFile(py) as z:
        z.extractall(folder / "python")
    pth = next((folder / "python").glob("python3*._pth"))
    pth.write_text("{0}\n.\n..\\lib\n".format(pth.name.replace("._pth", ".zip")))
    uv = fetch("https://github.com/astral-sh/uv/releases/download/{0}/uv-{1}.zip".format(UV, uv_target), "uv-{0}-{1}.zip".format(UV, uv_target))
    with zipfile.ZipFile(uv) as z:
        z.extractall(folder / "uv")
    for cmd in (ROOT / "packaging/windows").glob("*.cmd"):
        # Windows wants CR LF in batch files.
        (folder / cmd.name).write_bytes(cmd.read_text().replace("\r\n", "\n").replace("\n", "\r\n").encode())
    readme = (ROOT / "packaging/windows/README.txt").read_text()
    for key, value in {"VERSION": version(), "ARCH": arch, "PYTHON": PYTHON, "UV": UV, "POLARS": dict(WHEELS)["polars"], "WASMTIME": dict(WHEELS)["wasmtime"]}.items():
        readme = readme.replace("@{0}@".format(key), value)
    (folder / "README.txt").write_bytes(readme.replace("\n", "\r\n").encode())
    for lic in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy(ROOT / lic, folder / lic)
    target = DIST / (name + ".zip")
    if target.exists():
        target.unlink()
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(folder.rglob("*")):
            if f.is_file():
                z.write(f, Path(name) / f.relative_to(folder))
    say("  ", target.name, round(target.stat().st_size / 1e6, 1), "MB")
    return target


def build_skill():
    """skills/excel-port as a skill file for Claude (its folder inside the zip)."""
    target = DIST / "excel-port-skill.zip"
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted((ROOT / "skills/excel-port").rglob("*")):
            if f.is_file():
                z.write(f, Path("excel-port") / f.relative_to(ROOT / "skills/excel-port"))
    say("  ", target.name)


def checksums():
    lines = []
    for f in sorted(DIST.iterdir()):
        if f.is_file() and f.name != "SHA256SUMS":
            lines.append("{0}  {1}".format(hashlib.sha256(f.read_bytes()).hexdigest(), f.name))
    (DIST / "SHA256SUMS").write_text("\n".join(lines) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--source", help="the address of the repository, for the package's metadata")
    p.add_argument("--skip-wasm", action="store_true", help="take the module already built")
    p.add_argument("--only", choices=["wheel", "windows"])
    a = p.parse_args()
    if a.only == "windows":
        wheel = next(DIST.glob("xlcells-{0}-py3-none-any.whl".format(version())), None)
        if wheel is None:
            raise SystemExit("no wheel of this version in dist/: build it first")
    else:
        wheel = build_wheel(a.source, a.skip_wasm)
    build_skill()
    if a.only != "wheel":
        for arch in ARCHES:
            build_windows(wheel, arch)
    checksums()
    say("done:", ", ".join(sorted(f.name for f in DIST.iterdir())))


if __name__ == "__main__":
    sys.exit(main())
