"""Every XML part we write must be valid against the ISO/IEC 29500 schemas."""
import io
import zipfile

import pytest
from hypothesis import HealthCheck, given, settings
from lxml import etree

from conftest import CORPUS, ROOT
from test_conformance import DIR, text
from test_property import any_text, workbooks, write_ours

import struct
import xlcells

XSD = CORPUS / "ref-xsd"
pytestmark = pytest.mark.skipif(not (XSD / "sml.xsd").is_file(), reason="schemas not fetched (tools/fetch_corpus.sh)")
_schemas = {}


def schema(name):
    if name not in _schemas:
        _schemas[name] = etree.XMLSchema(etree.parse(str(XSD / name)))
    return _schemas[name]


def problems(data):
    """Schema violations of all parts of a written workbook."""
    out = []
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        assert z.testzip() is None
        names = z.namelist()
        assert names[0] == "[Content_Types].xml"
        for name in names:
            if name == "[Content_Types].xml":
                xsd = "opc-xsd/opc-contentTypes.xsd"
            elif name.endswith(".rels"):
                xsd = "opc-xsd/opc-relationships.xsd"
            elif name == "docProps/core.xml":
                out += [f"{name}: {e}" for e in core_properties_problems(z.read(name))]
                continue
            elif name == "docProps/custom.xml":
                xsd = "shared-documentPropertiesCustom.xsd"
            elif name == "docProps/app.xml":
                xsd = "shared-documentPropertiesExtended.xsd"
            else:
                xsd = "sml.xsd"
            doc = etree.fromstring(z.read(name))
            s = schema(xsd)
            if not s.validate(doc):
                # Excel writes <t xml:space="preserve"> for strings with outer whitespace
                # although the schema declares <t> as a plain string; so do we.
                errors = [e for e in s.error_log if not ("}t'" in e.message and "}space'" in e.message)]
                out += [f"{name}: {e.message} (line {e.line})" for e in errors[:3]]
    return out


CP = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
DC = "http://purl.org/dc/elements/1.1/"
DCTERMS = "http://purl.org/dc/terms/"
XSI = "http://www.w3.org/2001/XMLSchema-instance"
_CORE_CHILDREN = {(CP, n) for n in ("category", "contentStatus", "keywords", "lastModifiedBy", "lastPrinted", "revision", "version", "contentType")} | {
    (DC, n) for n in ("creator", "description", "identifier", "language", "subject", "title")} | {(DCTERMS, n) for n in ("created", "modified")}


def core_properties_problems(data):
    """The rules of ECMA-376 part 2, 11 for docProps/core.xml (its XSD needs the Dublin
    Core schemas, which are not fetched): root, allowed children each at most once,
    dates as W3CDTF, no markup compatibility."""
    root = etree.fromstring(data)
    out = []
    if root.tag != "{%s}coreProperties" % CP:
        out.append(f"root {root.tag}")
    seen = set()
    for child in root:
        q = etree.QName(child)
        key = (q.namespace, q.localname)
        if key not in _CORE_CHILDREN:
            out.append(f"element {child.tag} not allowed")
        if key in seen:
            out.append(f"element {child.tag} twice")
        seen.add(key)
        if q.namespace == DCTERMS and child.get("{%s}type" % XSI) != "dcterms:W3CDTF":
            out.append(f"{child.tag} without xsi:type dcterms:W3CDTF")
        if len(child):
            out.append(f"{child.tag} has child elements")
    return out


def conformance_files():
    for block in (DIR / "cases.txt").read_text(encoding="utf-8").split("\nend\n"):
        lines = block.split("\n")
        if not lines[0].startswith("case "):
            continue
        w = xlcells.Writer()
        styles, sheet = [0], 0
        for line in lines[1:]:
            f = line.split(" ")
            if f[0] == "date1904":
                w.set_date1904(f[1] == "1")
            elif f[0] == "style":
                styles.append(w.add_style(text(f[3]), f[1] == "1", f[2] == "1"))
            elif f[0] == "sheet":
                sheet = w.add_sheet(text(f[1]))
            elif f[0] == "merge":
                w.merge(sheet, *map(int, f[1:5]))
            elif f[0] == "width":
                w.set_col_width(sheet, int(f[1]), int(f[2]))
            elif f[0] == "cell":
                w.write(sheet, int(f[1]), int(f[2]), int(f[3]), struct.unpack(">d", bytes.fromhex(f[4]))[0], text(f[6]), text(f[7]), styles[int(f[5])])
        yield lines[0][5:], w.to_bytes()


def test_conformance_files_are_schema_valid():
    bad = {name: p for name, data in conformance_files() if (p := problems(data))}
    assert not bad, bad


@settings(max_examples=100, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(workbooks(any_text))
def test_random_workbooks_are_schema_valid(spec):
    assert not problems(write_ours(spec))


def test_formatted_workbook_is_schema_valid():
    import xlcells.openpyxl as facade
    from test_openpyxl_styles_and_keep import styled

    wb = styled(facade)
    assert not problems(wb.to_bytes())


def test_workbook_with_names_links_filter_and_validation_is_schema_valid():
    import xlcells.openpyxl as facade
    from test_openpyxl_objects import script

    assert not problems(script(facade).to_bytes())


def test_workbook_with_conditional_formatting_is_schema_valid():
    import xlcells.openpyxl as facade
    from test_openpyxl_conditional import script

    assert not problems(script(facade).to_bytes())


def test_workbook_with_print_settings_views_and_protection_is_schema_valid():
    from test_openpyxl_sheet_settings import configured, save
    import xlcells.openpyxl as facade
    assert problems(save(configured(facade))) == []
