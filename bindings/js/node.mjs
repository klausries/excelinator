// Entry point for Node.js.
import { createRequire } from 'node:module';
import { bind } from './compat.mjs';

export * from './compat.mjs';
const api = bind(createRequire(import.meta.url)('./pkg/node/xlcells.js'));
export const { Workbook, Worksheet, parse_xls, read, Writer, dateSerial } = api;
