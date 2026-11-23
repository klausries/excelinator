// The interface of pyExcelerator for JavaScript, on top of the WebAssembly core.
// `bind(core)` takes the generated module (pkg/node or pkg/web) and returns the API.

export const BLANK = 0, NUMBER = 1, TEXT = 2, BOOL = 3, ERROR = 4;

export class Formula {
  /** @param {string} text formula without '='; arguments separated by ';' or ',' */
  constructor(text, value = undefined) {
    this.text = text;
    this.value = value;
  }
}

export class ErrorCode {
  constructor(text) {
    this.text = text;
  }
}

export class XFStyle {
  constructor() {
    this.num_format_str = 'General';
    this.font = { bold: false, italic: false };
  }
}

export function bind(core) {
  const cell = (label) => {
    if (label instanceof Formula) {
      const [kind, num, text] = label.value === undefined ? [BLANK, 0, undefined] : cell(label.value);
      return [kind, num, text, label.text];
    }
    if (label instanceof ErrorCode) return [ERROR, 0, label.text, undefined];
    if (label === null || label === undefined || label === '') return [BLANK, 0, undefined, undefined];
    if (typeof label === 'boolean') return [BOOL, label ? 1 : 0, undefined, undefined];
    if (typeof label === 'string') return [TEXT, 0, label, undefined];
    if (typeof label === 'number') return [NUMBER, label, undefined, undefined];
    if (typeof label === 'bigint') return [NUMBER, Number(label), undefined, undefined];
    if (label instanceof Date) {
      // The wall-clock time of the Date in UTC is stored, as workbooks have no time zone.
      const serial = core.dateSerial(label.getUTCFullYear(), label.getUTCMonth() + 1, label.getUTCDate(), label.getUTCHours(), label.getUTCMinutes(), label.getUTCSeconds(), label.getUTCMilliseconds());
      if (serial === undefined) throw new RangeError(`date ${label.toISOString()} cannot be stored in a workbook`);
      return [NUMBER, serial, undefined, undefined];
    }
    throw new TypeError(`cannot write ${String(label)} into a cell`);
  };

  class Worksheet {
    constructor(workbook, name, index) {
      this.name = name;
      this._wb = workbook;
      this._index = index;
    }
    write(r, c, label = '', style = undefined) {
      const [kind, num, text, formula] = cell(label);
      this._wb._w.write(this._index, r, c, kind, num, text, formula, this._wb._style(style));
    }
    merge(r1, r2, c1, c2) {
      this._wb._w.merge(this._index, r1, r2, c1, c2);
    }
    write_merge(r1, r2, c1, c2, label = '', style = undefined) {
      this.merge(r1, r2, c1, c2);
      this.write(r1, c1, label, style);
    }
    /** width in 1/256 of a character, as `ws.col(c).width = w` in pyExcelerator */
    set_col_width(c, width) {
      this._wb._w.setColWidth(this._index, c, width);
    }
  }

  class Workbook {
    constructor() {
      this._w = new core.Writer();
      this._styles = new Map();
      this._sheets = [];
    }
    _style(style) {
      if (!style) return 0;
      const fmt = /^general$/i.test(style.num_format_str) ? 'General' : style.num_format_str;
      const key = JSON.stringify([fmt, !!style.font.bold, !!style.font.italic]);
      if (!this._styles.has(key)) this._styles.set(key, this._w.addStyle(fmt, !!style.font.bold, !!style.font.italic));
      return this._styles.get(key);
    }
    add_sheet(name) {
      const ws = new Worksheet(this, name, this._w.addSheet(name));
      this._sheets.push(ws);
      return ws;
    }
    get_sheet(index) {
      return this._sheets[index];
    }
    /** The xlsx file as bytes (pyExcelerator: save). */
    to_bytes() {
      return this._w.toBytes();
    }
  }

  /** [[sheetName, Map("row,col" → value)]]; dates arrive as numbers, errors as text. */
  function parse_xls(data) {
    const t = core.read(data);
    try {
      const sheets = t.sheetNames.map((name) => [name, new Map()]);
      const { sheet, row, col, kind, num, text } = t;
      for (let i = 0; i < t.length; i++) {
        const k = kind[i];
        if (k === BLANK) continue;
        const value = k === NUMBER ? num[i] : k === BOOL ? num[i] !== 0 : text[i];
        sheets[sheet[i]][1].set(`${row[i]},${col[i]}`, value);
      }
      return sheets;
    } finally {
      t.free();
    }
  }

  return { Workbook, Worksheet, Formula, ErrorCode, XFStyle, parse_xls, read: core.read, Writer: core.Writer, dateSerial: core.dateSerial };
}
