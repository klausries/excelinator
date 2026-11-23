/// The parts a screen is made of. They say what there is and what it does, not how it
/// looks: a surface draws each kind once (HTML, widgets) and the screens follow.
library;

/// What is shown at one moment: the head (where we are, what can be done with the
/// store), the list at the side, the content, the tabs at the foot (the sheets of a
/// workbook, as Excel has them), and what was said last.
class Screen {
  const Screen({required this.head, required this.side, required this.main, this.foot = const [], required this.messages, required this.busy});
  final List<Part> head;
  final List<Part> side;
  final List<Part> main;
  final List<Act> foot;
  final List<Message> messages;
  final bool busy;
}

class Message {
  Message(this.text, {this.error = false});
  final String text;
  final bool error;
}

sealed class Part {
  const Part();
}

/// How the parts of a group lie.
enum Lay {
  /// Across, one line with what belongs to a table or sheet (its name, its actions).
  bar,

  /// Across, the line that shows and takes the logic of what was picked.
  formula,

  /// Down a page that scrolls, with room around it.
  page,

  /// Down, each part a line of its own.
  lines,

  /// Along a line of text, wrapping.
  line,

  /// Buttons of which one is on.
  tabs,
}

class Parts extends Part {
  const Parts(this.lay, this.kids);
  final Lay lay;
  final List<Part> kids;
}

/// What a text is, which decides how it looks.
enum Tone {
  plain, brand, title, heading, strong, soft, hint, warn, badgeWarn, badgeOk, mono,

  /// What a check said, in lines as they came.
  report,

  /// Something a column reads.
  read,

  // The kinds of regions.
  table, cells, block,
}

class Words extends Part {
  const Words(this.text, [this.tone = Tone.plain, this.tip]);
  final String text;
  final Tone tone;
  final String? tip;
}

/// Room that takes what is left of a bar.
class Gap extends Part {
  const Gap();
}

/// Something to press: a button, or a link ([link]) to a place of the app ([href], for
/// surfaces that have addresses). Without [run] it cannot be pressed.
class Act extends Part {
  const Act(this.label, this.run, {this.tip, this.on = false, this.link = false, this.href, this.note, this.tone = Tone.plain});
  final String label;
  final void Function()? run;
  final String? tip;
  final bool on;
  final bool link;
  final String? href;

  /// A few words beside the label (the kind of a table in the list at the side).
  final String? note;
  final Tone tone;
}

/// A line of text to type. [done] gets it when the person is finished with it (Enter,
/// or leaving the field); without [done] it only shows.
class Entry extends Part {
  const Entry(this.value, this.done, {this.hint = '', this.tip, this.mono = false, this.wide = false, this.enterOnly = false});
  final String value;
  final void Function(String)? done;
  final String hint;
  final String? tip;
  final bool mono;

  /// Takes the room there is.
  final bool wide;

  /// Only Enter finishes; leaving the field leaves it be.
  final bool enterOnly;
}

/// One of a few: [options] are (value, label).
class Pick extends Part {
  const Pick(this.options, this.value, this.pick, {this.hint = ''});
  final List<(String, String)> options;
  final String? value;
  final void Function(String) pick;
  final String hint;
}

/// A small table of parts under headings (the files of the workspace, the columns of a
/// table with their logic).
class Listing extends Part {
  const Listing(this.head, this.rows, {this.wide = false});
  final List<String> head;
  final List<ListRow> rows;
  final bool wide;
}

class ListRow {
  const ListRow(this.cells, {this.on = false});
  final List<Part> cells;
  final bool on;
}

/// What a cell of a grid is, which decides its ground.
enum Ground { plain, calc, kept, own }

/// The rows of a table: columns with their sort and filter, rows by number, a foot of
/// sums. Rows and cells are asked for when they are drawn.
class DataGrid extends Part {
  const DataGrid({required this.cols, required this.rows, required this.head, required this.cell, required this.foot, this.more});
  final List<Col> cols;
  final int rows;
  final RowHead Function(int row) head;
  final Cell Function(int row, int col) cell;
  final List<Cell> foot;

  /// Loads further rows, where there are some.
  final Act? more;
}

class Col {
  const Col(this.name, {this.type = '', this.ground = Ground.plain, this.calc = false, this.tip, this.mark = '', required this.sort, this.filter});
  final String name;
  final String type;
  final Ground ground;
  final bool calc;
  final String? tip;

  /// ▲ or ▼ where the rows are sorted by this column.
  final String mark;

  /// Sorts by this column: up, down, not; [add] keeps the other columns sorted by.
  final void Function(bool add) sort;

  /// The column's filter, to type; null where it has none.
  final Entry? filter;
}

class RowHead {
  const RowHead(this.label, {this.picked = false, this.pick, this.tip});
  final String label;
  final String? tip;
  final bool picked;
  final void Function()? pick;
}

class Cell {
  const Cell(this.text, {this.number = false, this.ground = Ground.plain, this.tip, this.tap, this.edit, this.raw});
  final String text;
  final bool number;
  final Ground ground;
  final String? tip;
  final void Function()? tap;

  /// Takes the text typed in place of [raw]; null where the cell is not to be typed in.
  final void Function(String)? edit;
  final String? raw;
}

/// A part of a sheet as the workbook has it: cells with their look, the regions found
/// in it framed.
class SheetGrid extends Part {
  const SheetGrid({required this.rows, required this.cols, required this.colName, required this.width, required this.cell, required this.anchor, this.at, this.more, this.note});
  final int rows;
  final int cols;
  final String Function(int col) colName;

  /// The width of a column in points of the screen; null for the usual one.
  final double? Function(int col) width;

  /// The cell at a place; null where a merged cell beside or above covers it.
  final SheetCell? Function(int row, int col) cell;

  /// For a place a merged cell covers, where that cell is; null for any other.
  final (int, int)? Function(int row, int col) anchor;

  /// The cell picked: (row, column); its row and column are marked in the heads.
  final (int, int)? at;
  final Act? more;
  final String? note;
}

class SheetCell {
  const SheetCell(this.text, {this.bold = false, this.italic = false, this.fill, this.color, this.align, this.rowSpan = 1, this.colSpan = 1, this.region, this.edges = '', this.calc = false, this.picked = false, this.tip, this.tap});
  final String text;
  final bool bold;
  final bool italic;

  /// Colours as `#rrggbb`.
  final String? fill;
  final String? color;

  /// `left`, `center`, `right`; null: as the surface would.
  final String? align;
  final int rowSpan;
  final int colSpan;

  /// The kind of the region the cell lies in ([Tone.table], [Tone.cells], [Tone.block]).
  final Tone? region;

  /// The sides at which the region ends here: letters of `tblr`.
  final String edges;
  final bool calc;
  final bool picked;
  final String? tip;
  final void Function()? tap;
}
