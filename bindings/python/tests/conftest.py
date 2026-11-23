import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
CORPUS = pathlib.Path(os.environ.get("XLCELLS_CORPUS", ROOT / "corpus"))

import pytest  # noqa: E402

#: Tests on the corpus of real workbooks (tools/fetch_corpus.sh, several GB): skipped
#: where it is not fetched, as in CI.
needs_corpus = pytest.mark.skipif(not any(CORPUS.glob("src-*")), reason="corpus not fetched (tools/fetch_corpus.sh)")
