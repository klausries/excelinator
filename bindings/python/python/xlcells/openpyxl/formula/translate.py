"""Move a formula to another cell (`Translator("=A1+$B$1", origin="C3").translate_formula("D5")`).

Relative references move with the formula, absolute ones stay; a reference that would
leave the sheet raises `TranslatorError`, as with openpyxl.
"""
from ... import _xlcells
from ..utils.cell import coordinate_to_tuple

__all__ = ["Translator", "TranslatorError"]


class TranslatorError(ValueError):
    """A reference would leave the sheet."""


class Translator:
    def __init__(self, formula, origin):
        self.formula = formula
        self.origin = origin

    def get_tokens(self):
        from .tokenizer import Tokenizer
        return Tokenizer(self.formula).items

    def translate_formula(self, dest=None, row_delta=0, col_delta=0):
        """The formula as it reads in `dest` (or moved by the deltas)."""
        text = self.formula
        if not text.startswith("="):
            return text
        if dest is not None:
            (r0, c0), (r1, c1) = coordinate_to_tuple(self.origin.split("!")[-1]), coordinate_to_tuple(dest.split("!")[-1])
            row_delta, col_delta = r1 - r0, c1 - c0
        moved = _xlcells.translate_formula(text[1:], row_delta, col_delta)
        if moved is None:
            raise TranslatorError("Formula cannot be read: {0}".format(text))
        if moved.count("#REF!") > text.count("#REF!"):
            raise TranslatorError("Formula out of range")
        return "=" + moved

    @staticmethod
    def translate_row(row_str, rdelta):
        if row_str.startswith("$"):
            return row_str
        row = int(row_str) + rdelta
        if row <= 0:
            raise TranslatorError("Formula out of range")
        return str(row)

    @staticmethod
    def translate_col(col_str, cdelta):
        from ..utils.cell import column_index_from_string, get_column_letter
        if col_str.startswith("$"):
            return col_str
        try:
            return get_column_letter(column_index_from_string(col_str) + cdelta)
        except ValueError:
            raise TranslatorError("Formula out of range")
