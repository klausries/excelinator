#!/bin/sh
# Fetch the reference corpora into corpus/. Sources and licences: see docs/TECHNIK.md.
set -e
mkdir -p corpus && cd corpus
[ -d src-rust_xlsxwriter ] || git clone --depth 1 https://github.com/jmcnamara/rust_xlsxwriter src-rust_xlsxwriter
[ -d src-calamine ] || git clone --depth 1 https://github.com/tafia/calamine src-calamine
if [ ! -d src-poi ]; then
  git clone --depth 1 --filter=blob:none --sparse https://github.com/apache/poi src-poi
  (cd src-poi && git sparse-checkout set test-data/spreadsheet)
fi
[ -d src-enron ] || { git clone --depth 1 https://github.com/SheetJS/enron_xls src-enron && rm -rf src-enron/.git; }
cd ..
# Legacy .xls files recalculated by LibreOffice (needs only soffice: the tools drive it
# with a Basic macro, see tools/lo_basic.py).
legacy=$(mktemp -d) && cp -r corpus/src-poi corpus/src-calamine "$legacy/"
python3 tools/lo_oracle.py "$legacy" corpus/lo-oracle xls || echo "LibreOffice oracle skipped"
rm -rf "$legacy"
python3 tools/lo_oracle.py corpus/src-enron corpus/lo-enron xls 14 || echo "Enron sample skipped"
# The look of sampled cells of every xlsx, as LibreOffice formats them (golden of xl-look).
python3 tools/lo_look.py corpus corpus/lo-look || echo "LibreOffice looks skipped"
cargo run --release -q -p xl-golden --bin xlg -- manifest

# pyExcelerator 0.6.4.1 (BSD with advertising clause): reference for the compatible
# interface. Its example scripts are run by the tests; nothing of it is copied into
# this repository.
if [ ! -d corpus/ref-pyexcelerator ]; then
  mkdir -p corpus/ref-pyexcelerator /tmp/pyx-dl
  pip download pyExcelerator==0.6.4.1 -d /tmp/pyx-dl --no-deps --no-binary :all:
  tar xjf /tmp/pyx-dl/pyexcelerator-0.6.4.1.tar.bz2 -C corpus/ref-pyexcelerator --strip-components=1
fi

# ISO/IEC 29500-4 (transitional) XML schemas, for validating the parts we write.
if [ ! -d corpus/ref-xsd ]; then
  git clone -q --depth 1 --filter=blob:none --sparse https://github.com/python-openxml/python-docx /tmp/python-docx-xsd
  (cd /tmp/python-docx-xsd && git sparse-checkout set ref/xsd)
  mkdir -p corpus/ref-xsd && cp -r /tmp/python-docx-xsd/ref/xsd/. corpus/ref-xsd/
fi
