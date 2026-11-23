import type { Table, Writer } from './pkg/node/xlcells.js';

export const BLANK: 0, NUMBER: 1, TEXT: 2, BOOL: 3, ERROR: 4;
export type CellValue = number | string | boolean;
export type Label = CellValue | bigint | Date | Formula | ErrorCode | null | undefined;

export class Formula {
  constructor(text: string, value?: CellValue | ErrorCode);
  text: string;
  value?: CellValue | ErrorCode;
}
export class ErrorCode {
  constructor(text: string);
  text: string;
}
export class XFStyle {
  num_format_str: string;
  font: { bold: boolean; italic: boolean };
}
export interface Worksheet {
  readonly name: string;
  write(r: number, c: number, label?: Label, style?: XFStyle): void;
  merge(r1: number, r2: number, c1: number, c2: number): void;
  write_merge(r1: number, r2: number, c1: number, c2: number, label?: Label, style?: XFStyle): void;
  set_col_width(c: number, width: number): void;
}
export interface Workbook {
  add_sheet(name: string): Worksheet;
  get_sheet(index: number): Worksheet;
  to_bytes(): Uint8Array;
}
export interface Api {
  Workbook: new () => Workbook;
  Formula: typeof Formula;
  ErrorCode: typeof ErrorCode;
  XFStyle: typeof XFStyle;
  /** Per sheet its name and a map from "row,col" to the value. */
  parse_xls(data: Uint8Array): [string, Map<string, CellValue>][];
  read(data: Uint8Array): Table;
  Writer: typeof Writer;
  dateSerial(year: number, month: number, day: number, hour: number, minute: number, second: number, millisecond: number): number | undefined;
}
export function bind(core: unknown): Api;
