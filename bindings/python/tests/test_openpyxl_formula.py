"""Formula tokenizer and translator against openpyxl's, over every formula in the corpus."""
import random
import re
import zipfile

import pytest
from openpyxl.formula.tokenizer import Tokenizer as TheirTokenizer
from openpyxl.formula.translate import Translator as TheirTranslator

from conftest import CORPUS
from xlcells.openpyxl.formula import Tokenizer
from xlcells.openpyxl.formula.translate import Translator, TranslatorError

_F = re.compile(r"<f[^>]*>([^<]+)</f>")
_ENTITIES = (("&lt;", "<"), ("&gt;", ">"), ("&quot;", '"'), ("&apos;", "'"), ("&amp;", "&"))


def corpus_formulas(limit=2000):
    found = set()
    for path in sorted(CORPUS.glob("src-*/**/*.xlsx"))[:limit]:
        try:
            with zipfile.ZipFile(path) as z:
                for name in z.namelist():
                    if name.startswith("xl/worksheets/") and name.endswith(".xml"):
                        for f in _F.findall(z.read(name).decode("utf8", "replace")):
                            for a, b in _ENTITIES:
                                f = f.replace(a, b)
                            found.add("=" + f)
        except (zipfile.BadZipFile, OSError):
            continue
    return sorted(found)


FORMULAS = corpus_formulas()
pytestmark = pytest.mark.skipif(len(FORMULAS) < 1000, reason="corpus not fetched (tools/fetch_corpus.sh)")


def tokens(cls, formula):
    try:
        return [(t.value, t.type, t.subtype) for t in cls(formula).items]
    except Exception as e:  # the same failure counts as agreement
        return type(e).__name__


def test_tokens_are_openpyxls_for_every_corpus_formula():
    differ = [f for f in FORMULAS if tokens(TheirTokenizer, f) != tokens(Tokenizer, f)]
    assert not differ, differ[:10]
    assert Tokenizer("=SUM(A1:B2)").render() == "=SUM(A1:B2)"


def test_translated_formulas_are_openpyxls_where_openpyxl_gets_them_right():
    """openpyxl folds runs of white space into one character, takes line breaks apart and
    drops the sheet or workbook in front of a defined name; we keep all of them, and
    otherwise agree."""
    rng = random.Random(7)
    space = lambda s: re.sub(r"[ \r\n]+", " ", s)  # noqa: E731
    checked = 0
    for f in FORMULAS:
        dr, dc = rng.randint(-3, 40), rng.randint(-2, 10)
        try:
            theirs = TheirTranslator(f, "C5").translate_formula(row_delta=dr, col_delta=dc)
        except Exception:
            continue  # openpyxl cannot (out of the sheet, or a formula its tokenizer cannot split)
        ours = Translator(f, "C5").translate_formula(row_delta=dr, col_delta=dc)
        if theirs.count("!") < f.count("!") or theirs == "=" or "\r" in f or "\n" in f:
            continue  # openpyxl lost a prefix or split a line break
        assert space(ours) == space(theirs), (f, dr, dc)
        checked += 1
    assert checked > 1000


def test_translator_details():
    assert Translator("=SUM(A1:B2)+$C$3", "A1").translate_formula("B3") == "=SUM(B3:C4)+$C$3"
    assert Translator("=A:A+1:1", "A1").translate_formula(row_delta=2, col_delta=1) == "=B:B+3:3"
    assert Translator("=Sheet2!A1*'My Sheet'!B$2", "A1").translate_formula(row_delta=1) == "=Sheet2!A2*'My Sheet'!B$2"
    assert Translator('=IF(A1>0,"A1",B1)', "A1").translate_formula(col_delta=1) == '=IF(B1>0,"A1",C1)'
    with pytest.raises(TranslatorError):
        Translator("=A1", "B2").translate_formula(row_delta=-1)
    assert Translator("not a formula", "A1").translate_formula("C3") == "not a formula"
