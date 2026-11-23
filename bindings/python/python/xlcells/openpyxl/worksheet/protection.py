"""Sheet protection (`ws.protection`) and Excel's password hash."""
from .._serial import Element

__all__ = ["SheetProtection", "hash_password"]


def hash_password(plaintext_password=""):
    """Excel's legacy 16-bit password verifier (ECMA-376 part 4, 14.7.1), as hex text."""
    value = 0
    for position, char in enumerate(plaintext_password, 1):
        shifted = ord(char) << position
        value ^= (shifted & 0x7FFF) | (shifted >> 15)
    value ^= len(plaintext_password)
    value ^= 0xCE4B
    return format(value, "X")


class SheetProtection(Element):
    tagname = "sheetProtection"
    _attrs = (
        ("selectLockedCells", bool, False), ("selectUnlockedCells", bool, False), ("algorithmName", str, None), ("sheet", bool, False),
        ("objects", bool, False), ("insertRows", bool, True), ("insertHyperlinks", bool, True), ("autoFilter", bool, True),
        ("scenarios", bool, False), ("formatColumns", bool, True), ("deleteColumns", bool, True), ("insertColumns", bool, True),
        ("pivotTables", bool, True), ("deleteRows", bool, True), ("formatCells", bool, True), ("saltValue", str, None),
        ("formatRows", bool, True), ("sort", bool, True), ("spinCount", int, None), ("password", str, None), ("hashValue", str, None),
    )

    def __init__(self, *args, **kw):
        password = kw.pop("password", None)
        Element.__init__(self, *args, **kw)
        if password is not None:
            self.set_password(password)

    def set_password(self, value="", already_hashed=False):
        """Protect the sheet with a password (stored as Excel's hash)."""
        object.__setattr__(self, "password", value if already_hashed else hash_password(value))
        self.enable()

    def __setattr__(self, name, value):
        if name == "password" and value is not None:
            self.set_password(value)
            return
        object.__setattr__(self, name, value)

    def _xml(self, tag=None):
        # As openpyxl: an unprotected sheet has no protection element.
        return Element._xml(self, tag) if self.sheet else ""

    def enable(self):
        self.sheet = True

    def disable(self):
        self.sheet = False

    @property
    def enabled(self):
        return bool(self.sheet)

    def __bool__(self):
        return bool(self.sheet)
