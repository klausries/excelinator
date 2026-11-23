"""Workbook settings: `workbookPr` (`WorkbookProperties`, with `wb.code_name`) and
calculation (`wb.calculation`, `CalcProperties`)."""
from .._serial import Element

__all__ = ["WorkbookProperties", "CalcProperties", "FileVersion"]


class WorkbookProperties(Element):
    tagname = "workbookPr"
    _attrs = (
        ("date1904", bool, None), ("dateCompatibility", bool, None), ("showObjects", str, None), ("showBorderUnselectedTables", bool, None),
        ("filterPrivacy", bool, None), ("promptedSolutions", bool, None), ("showInkAnnotation", bool, None), ("backupFile", bool, None),
        ("saveExternalLinkValues", bool, None), ("updateLinks", str, None), ("codeName", str, None), ("hidePivotFieldList", bool, None),
        ("showPivotChartFilter", bool, None), ("allowRefreshQuery", bool, None), ("publishItems", bool, None), ("checkCompatibility", bool, None),
        ("autoCompressPictures", bool, None), ("refreshAllConnections", bool, None), ("defaultThemeVersion", int, None),
    )


class CalcProperties(Element):
    """How Excel calculates: mode, iteration, precision, recalculation on load."""

    tagname = "calcPr"
    _attrs = (
        ("calcId", int, 124519), ("calcMode", str, None), ("fullCalcOnLoad", bool, True), ("refMode", str, None), ("iterate", bool, None),
        ("iterateCount", int, None), ("iterateDelta", float, None), ("fullPrecision", bool, None), ("calcCompleted", bool, None),
        ("calcOnSave", bool, None), ("concurrentCalc", bool, None), ("concurrentManualCount", int, None), ("forceFullCalc", bool, None),
    )


class FileVersion(Element):
    tagname = "fileVersion"
    _attrs = (("appName", str, None), ("lastEdited", str, None), ("lowestEdited", str, None), ("rupBuild", str, None), ("codeName", str, None))
