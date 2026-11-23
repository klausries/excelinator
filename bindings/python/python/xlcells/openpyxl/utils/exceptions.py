"""The exceptions of openpyxl's `utils.exceptions`."""
from .cell import CellCoordinatesException  # noqa: F401

__all__ = ["CellCoordinatesException", "IllegalCharacterError", "NamedRangeException", "SheetTitleException", "InvalidFileException",
           "ReadOnlyWorkbookException", "WorkbookAlreadySaved"]


class NamedRangeException(Exception):
    """Error for badly formatted named ranges."""


class SheetTitleException(Exception):
    """Error for bad sheet names."""


class InvalidFileException(Exception):
    """Error for trying to open a file that is not an xlsx file."""


class ReadOnlyWorkbookException(Exception):
    """Error for trying to modify a read-only workbook."""


class WorkbookAlreadySaved(Exception):
    """Error when attempting to perform operations on a dump workbook while it has
    already been dumped once."""


def __getattr__(name):
    if name == "IllegalCharacterError":
        from .. import IllegalCharacterError
        return IllegalCharacterError
    raise AttributeError(name)
