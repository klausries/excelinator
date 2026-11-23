"""The conditional formatting of a sheet: rules per cell range (openpyxl's
`formatting.formatting`)."""
from ..worksheet.cell_range import MultiCellRange
from .rule import Rule

__all__ = ["ConditionalFormatting", "ConditionalFormattingList"]


class ConditionalFormatting:
    """The rules for one set of ranges (`sqref`, alias `cells`)."""

    tagname = "conditionalFormatting"

    def __init__(self, sqref=(), pivot=None, cfRule=(), extLst=None):
        self.sqref = sqref if isinstance(sqref, MultiCellRange) else MultiCellRange(sqref)
        self.pivot = pivot
        self.cfRule = list(cfRule)

    rules = property(lambda self: self.cfRule, lambda self, v: setattr(self, "cfRule", v))
    cells = property(lambda self: self.sqref)

    def __eq__(self, other):
        return isinstance(other, ConditionalFormatting) and self.sqref == other.sqref

    def __hash__(self):
        return hash(str(self.sqref))

    def __repr__(self):
        return "<{0} {1}>".format(type(self).__name__, self.sqref)

    def __contains__(self, coord):
        return coord in self.sqref


class ConditionalFormattingList:
    """Conditional formatting rules of a sheet, in the order they were added."""

    def __init__(self):
        self._cf_rules = {}
        self.max_priority = 0

    def add(self, range_string, cfRule):
        """Add a rule (ColorScaleRule, FormulaRule, CellIsRule, ...); without a priority
        it gets the next one."""
        cf = ConditionalFormatting(range_string) if isinstance(range_string, str) else range_string
        if not isinstance(cfRule, Rule):
            raise ValueError("Only instances of openpyxl.formatting.rule.Rule may be added")
        self.max_priority += 1
        if not cfRule.priority:
            cfRule.priority = self.max_priority
        self._cf_rules.setdefault(cf, []).append(cfRule)

    def __bool__(self):
        return bool(self._cf_rules)

    def __len__(self):
        return len(self._cf_rules)

    def __iter__(self):
        for cf, rules in self._cf_rules.items():
            cf.rules = rules
            yield cf

    def __getitem__(self, key):
        if isinstance(key, str):
            key = ConditionalFormatting(sqref=key)
        return self._cf_rules[key]

    def __delitem__(self, key):
        del self._cf_rules[ConditionalFormatting(sqref=key)]

    def __setitem__(self, key, rule):
        self.add(key, rule)
