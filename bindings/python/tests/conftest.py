import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
CORPUS = pathlib.Path(os.environ.get("XLCELLS_CORPUS", ROOT / "corpus"))
