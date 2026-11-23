/// Read and write Excel workbooks (xlsx) as cell tables.
///
/// [read] returns the cells as equally long columns, [Writer] builds a workbook cell
/// by cell, and [Workbook]/[parseXls] offer the same in the shape of pyExcelerator.
library;

export 'src/core.dart' show Writer, Table, read, dateSerial, XlError, CellKind, loadLibrary;
export 'src/compat.dart';
