"""Workbook protection (`wb.security`): structure, windows and revisions, with Excel's
legacy password hash or a hash given by name, salt and spin count."""
from .._serial import Element
from ..worksheet.protection import hash_password

__all__ = ["WorkbookProtection", "DocumentSecurity", "FileSharing"]


class WorkbookProtection(Element):
    tagname = "workbookProtection"
    _attrs = (
        ("workbookPassword", str, None), ("workbookPasswordCharacterSet", str, None), ("revisionsPassword", str, None),
        ("revisionsPasswordCharacterSet", str, None), ("lockStructure", bool, None), ("lockWindows", bool, None), ("lockRevision", bool, None),
        ("revisionsAlgorithmName", str, None), ("revisionsHashValue", str, None), ("revisionsSaltValue", str, None), ("revisionsSpinCount", int, None),
        ("workbookAlgorithmName", str, None), ("workbookHashValue", str, None), ("workbookSaltValue", str, None), ("workbookSpinCount", int, None),
    )

    def __init__(self, *args, **kw):
        workbook, revisions = kw.pop("workbookPassword", None), kw.pop("revisionsPassword", None)
        Element.__init__(self, *args, **kw)
        if workbook is not None:
            self.workbookPassword = workbook
        if revisions is not None:
            self.revisionsPassword = revisions

    def __setattr__(self, key, value):
        # As openpyxl: a password set by attribute is hashed.
        if key in ("workbookPassword", "revisionsPassword") and value is not None:
            value = hash_password(value)
        object.__setattr__(self, key, value)

    def set_workbook_password(self, value="", already_hashed=False):
        """Protect with a password; `already_hashed` takes the stored form."""
        object.__setattr__(self, "workbookPassword", value if already_hashed else hash_password(value))

    @property
    def workbook_password(self):
        return self.workbookPassword

    @workbook_password.setter
    def workbook_password(self, value):
        self.set_workbook_password(value)

    def set_revisions_password(self, value="", already_hashed=False):
        object.__setattr__(self, "revisionsPassword", value if already_hashed else hash_password(value))

    @property
    def revision_password(self):
        return self.revisionsPassword

    @revision_password.setter
    def revision_password(self, value):
        self.set_revisions_password(value)

    @classmethod
    def from_tree(cls, node):
        import xml.etree.ElementTree as ET
        return cls._from_xml(ET.tostring(node, encoding="unicode"))


DocumentSecurity = WorkbookProtection


class FileSharing(Element):
    tagname = "fileSharing"
    _attrs = (("readOnlyRecommended", bool, None), ("userName", str, None), ("reservationPassword", str, None), ("algorithmName", str, None),
              ("hashValue", str, None), ("saltValue", str, None), ("spinCount", int, None))
