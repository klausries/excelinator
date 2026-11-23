/// The screens of the app as widgets: each kind of part is drawn here once.
library;

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:two_dimensional_scrollables/two_dimensional_scrollables.dart';
import 'package:xl_app_core/xl_app_core.dart';

Color hex(String s) => Color(0xff000000 | int.parse(s.substring(1), radix: 16));

/// A colour of the app's list (see the core's palette), light or dark as the theme is.
Color tint(BuildContext context, String name) => hex((Theme.of(context).brightness == Brightness.dark ? dark : light)[name]!);

const mono = TextStyle(fontFamily: 'Menlo', fontFamilyFallback: ['Consolas', 'monospace']);

/// The app in a window: head, side, content, and what was said.
class Surface extends StatefulWidget {
  const Surface(this.app, {this.extra = const [], super.key});
  final App app;

  /// What this surface adds to the head (its folder).
  final List<Widget> extra;

  @override
  State<Surface> createState() => _SurfaceState();
}

class _SurfaceState extends State<Surface> {
  @override
  void initState() {
    super.initState();
    widget.app.listen(() {
      if (mounted) setState(() {});
    });
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.app.screen;
    final line = tint(context, 'line'), panel = tint(context, 'panel');
    bool fills(Part p) => p is DataGrid || p is SheetGrid || (p is Parts && p.lay == Lay.page);
    // The head in two lines, as Excel has it: the title bar (the app, the store), then
    // the levels as tabs with what can be done on the right.
    final tabs = s.head.indexWhere((p) => p is Parts && p.lay == Lay.tabs);
    final title = tabs < 0 ? s.head : s.head.sublist(0, tabs);
    final ribbon = tabs < 0 ? const <Part>[] : s.head.sublist(tabs);
    final last = s.messages.lastOrNull;
    return Scaffold(
      backgroundColor: tint(context, 'bg'),
      body: Stack(
        children: [
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _titleBar(context, title, widget.extra),
              if (ribbon.isNotEmpty) _ribbon(context, ribbon),
              Expanded(
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    if (s.side.isNotEmpty)
                      Container(
                        width: 220,
                        decoration: BoxDecoration(border: Border(right: BorderSide(color: line))),
                        child: ListView(padding: const EdgeInsets.only(bottom: 12), children: [for (final p in s.side) _sideItem(context, p)]),
                      ),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (final p in s.main) fills(p) ? Expanded(child: draw(context, p)) : draw(context, p),
                          if (s.foot.isNotEmpty) _sheetTabs(context, s.foot),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
              // The status bar.
              Container(
                height: 24,
                padding: const EdgeInsets.symmetric(horizontal: 12),
                alignment: Alignment.centerLeft,
                decoration: BoxDecoration(color: panel, border: Border(top: BorderSide(color: line))),
                child: Text(
                  s.busy ? 'Arbeitet …' : last?.text ?? 'Bereit',
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 12, color: last?.error == true && !s.busy ? tint(context, 'error') : tint(context, 'soft')),
                ),
              ),
            ],
          ),
          if (s.busy) const Positioned(top: 0, left: 0, right: 0, child: LinearProgressIndicator(minHeight: 2)),
          Positioned(
            right: 14,
            bottom: 34,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                for (final m in s.messages)
                  Container(
                    margin: const EdgeInsets.only(top: 6),
                    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                    constraints: const BoxConstraints(maxWidth: 460),
                    decoration: BoxDecoration(color: m.error ? tint(context, 'error') : tint(context, 'text'), borderRadius: BorderRadius.circular(4), boxShadow: kElevationToShadow[3]),
                    child: Text(m.text, style: TextStyle(color: m.error ? Colors.white : tint(context, 'bg'))),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The title bar in the green of the app: its name, the store, the way to the start.
Widget _titleBar(BuildContext context, List<Part> parts, List<Widget> extra) {
  final theme = Theme.of(context), on = tint(context, 'onBrand');
  return Theme(
    data: theme.copyWith(
      canvasColor: tint(context, 'brand'),
      textTheme: theme.textTheme.apply(bodyColor: on, displayColor: on),
      outlinedButtonTheme: OutlinedButtonThemeData(style: OutlinedButton.styleFrom(foregroundColor: on, side: BorderSide.none)),
      textButtonTheme: TextButtonThemeData(style: TextButton.styleFrom(foregroundColor: on)),
    ),
    child: Builder(
      builder: (context) => DefaultTextStyle.merge(
        style: TextStyle(color: on),
        child: Container(
          height: 40,
          color: tint(context, 'brand'),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          child: Row(
            children: [
              Icon(Icons.grid_on, size: 16, color: on),
              const SizedBox(width: 7),
              for (final p in parts) Padding(padding: const EdgeInsets.only(right: 10), child: draw(context, p)),
              const Spacer(),
              ...extra,
            ],
          ),
        ),
      ),
    ),
  );
}

/// The ribbon: the levels as tabs, flat buttons on the right.
Widget _ribbon(BuildContext context, List<Part> parts) => Container(
  height: 40,
  padding: const EdgeInsets.symmetric(horizontal: 6),
  decoration: BoxDecoration(color: tint(context, 'panel'), border: Border(bottom: BorderSide(color: tint(context, 'line')))),
  child: Row(
    children: [
      for (final p in parts)
        switch (p) {
          Parts(lay: Lay.tabs) => Row(children: [for (final t in p.kids.whereType<Act>()) _tab(context, t)]),
          Gap() => const Spacer(),
          Act() => _tip(p.tip, _button(context, p, flat: true)),
          _ => Padding(padding: const EdgeInsets.symmetric(horizontal: 6), child: draw(context, p)),
        },
    ],
  ),
);

/// A tab: a word, green and underlined where it is on.
Widget _tab(BuildContext context, Act a, {bool foot = false}) {
  final accent = tint(context, 'accent');
  return InkWell(
    onTap: a.run,
    child: Container(
      height: foot ? 30 : 40,
      padding: const EdgeInsets.symmetric(horizontal: 14),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: foot && a.on ? tint(context, 'bg') : null,
        border: Border(
          bottom: a.on ? BorderSide(color: accent, width: 3) : BorderSide.none,
          right: foot ? BorderSide(color: tint(context, 'line')) : BorderSide.none,
        ),
      ),
      child: Text(
        a.label,
        style: TextStyle(color: a.on ? accent : a.run == null ? tint(context, 'soft') : null, fontWeight: a.on ? FontWeight.w600 : null),
      ),
    ),
  );
}

/// The sheets of the workbook as tabs at its foot.
Widget _sheetTabs(BuildContext context, List<Act> tabs) => Container(
  height: 30,
  padding: const EdgeInsets.only(left: 46),
  decoration: BoxDecoration(color: tint(context, 'panel'), border: Border(top: BorderSide(color: tint(context, 'line')))),
  child: ListView(scrollDirection: Axis.horizontal, children: [for (final a in tabs) _tip(a.tip, _tab(context, a, foot: true))]),
);

/// A table or sheet in the list at the side, or a heading of the list.
Widget _sideItem(BuildContext context, Part p) {
  if (p is! Act) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 14, 12, 4),
      child: Text((p as Words).text.toUpperCase(), style: TextStyle(fontSize: 11, fontWeight: FontWeight.w600, letterSpacing: .5, color: tint(context, 'soft'))),
    );
  }
  return InkWell(
    onTap: p.run,
    child: Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
      decoration: BoxDecoration(
        color: p.on ? tint(context, 'pick') : null,
        border: Border(left: BorderSide(color: p.on ? tint(context, 'accent') : Colors.transparent, width: 3)),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(p.label, overflow: TextOverflow.ellipsis, style: TextStyle(fontWeight: p.on ? FontWeight.w600 : FontWeight.w400)),
          ),
          if (p.note != null) Text(p.note!, style: TextStyle(fontSize: 11, color: tint(context, 'soft'))),
        ],
      ),
    ),
  );
}

/// Parts across a bar: what comes before a gap on the left, the rest on the right; a
/// wide field takes the room there is.
Widget _across(BuildContext context, List<Part> kids, [List<Widget> extra = const []]) {
  if (kids.any((k) => k is Entry && k.wide)) {
    return Row(
      children: [
        for (final k in kids)
          k is Entry && k.wide
              ? Expanded(
                  child: Padding(padding: const EdgeInsets.symmetric(horizontal: 8), child: Field(k, expand: true)),
                )
              : Padding(padding: const EdgeInsets.only(right: 6), child: draw(context, k)),
      ],
    );
  }
  final gap = kids.indexWhere((k) => k is Gap);
  Widget wrap(Iterable<Part> parts, [List<Widget> more = const []]) => Wrap(spacing: 8, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [for (final k in parts) draw(context, k), ...more]);
  // Side by side where there is room, one under the other where there is not.
  return SizedBox(
    width: double.infinity,
    child: Wrap(
      alignment: WrapAlignment.spaceBetween,
      runSpacing: 6,
      spacing: 12,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [wrap(gap < 0 ? kids : kids.take(gap)), if (gap >= 0 || extra.isNotEmpty) wrap(gap < 0 ? const [] : kids.skip(gap + 1), extra)],
    ),
  );
}

Widget _tip(String? tip, Widget child) => tip == null || tip.isEmpty ? child : Tooltip(message: tip, waitDuration: const Duration(milliseconds: 500), child: child);

/// A few words with a line around them.
Widget _chip(BuildContext context, String text, Color color, {Color? ground, Color? line, double radius = 10}) => Container(
  padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
  decoration: BoxDecoration(
    color: ground,
    borderRadius: BorderRadius.circular(radius),
    border: Border.all(color: line ?? color),
  ),
  child: Text(text, style: TextStyle(fontSize: 12, color: color)),
);

Widget _words(BuildContext context, String text, Tone tone) {
  final soft = tint(context, 'soft');
  return switch (tone) {
    Tone.brand => Text(text, style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w700)),
    Tone.title => Text(text, style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600)),
    Tone.heading => Padding(padding: const EdgeInsets.only(top: 10), child: Text(text, style: const TextStyle(fontWeight: FontWeight.w600))),
    Tone.strong => Text(text, style: const TextStyle(fontWeight: FontWeight.w600)),
    Tone.soft => Text(text, style: TextStyle(fontSize: 12, color: soft)),
    Tone.hint => Text(text, style: TextStyle(color: soft)),
    Tone.warn => Text(text, style: TextStyle(fontSize: 12, color: tint(context, 'warn'))),
    Tone.badgeWarn => _chip(context, text, tint(context, 'warn'), ground: tint(context, 'kept')),
    Tone.badgeOk => _chip(context, text, tint(context, 'ok')),
    Tone.mono => Text(text, style: mono),
    Tone.report => Container(
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: tint(context, 'panel'),
        borderRadius: BorderRadius.circular(5),
        border: Border.all(color: tint(context, 'line')),
      ),
      child: SelectableText(text, style: mono),
    ),
    Tone.read => _chip(context, text, tint(context, 'text'), line: tint(context, 'line'), radius: 4),
    Tone.table || Tone.cells || Tone.block => _chip(context, text, tint(context, tone.name)),
    Tone.plain => Text(text),
  };
}

Widget _button(BuildContext context, Act a, {bool flat = false}) {
  final style = ButtonStyle(
    visualDensity: VisualDensity.compact,
    textStyle: const WidgetStatePropertyAll(TextStyle(fontSize: 13, fontWeight: FontWeight.w400)),
    padding: const WidgetStatePropertyAll(EdgeInsets.symmetric(horizontal: 10)),
    minimumSize: const WidgetStatePropertyAll(Size(0, 28)),
    shape: WidgetStatePropertyAll(RoundedRectangleBorder(borderRadius: BorderRadius.circular(4))),
  );
  if (flat) return TextButton(onPressed: a.run, style: style.copyWith(foregroundColor: WidgetStatePropertyAll(a.run == null ? tint(context, 'soft') : tint(context, 'text'))), child: Text(a.label));
  return a.on
      ? FilledButton(onPressed: a.run, style: style, child: Text(a.label))
      : OutlinedButton(onPressed: a.run, style: style.copyWith(foregroundColor: WidgetStatePropertyAll(a.run == null ? tint(context, 'soft') : tint(context, 'text'))), child: Text(a.label));
}

Widget draw(BuildContext context, Part p) => switch (p) {
  // The formula bar: the address in a name box, as in Excel.
  Parts(lay: Lay.formula, kids: [Words(tone: Tone.strong) && final name, ...final rest]) => draw(
    context,
    Parts(Lay.bar, [_NameBox(name.text), ...rest]),
  ),
  Parts(lay: Lay.bar || Lay.formula) => Container(
    constraints: const BoxConstraints(minHeight: 40),
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
    decoration: BoxDecoration(
      border: Border(bottom: BorderSide(color: tint(context, 'line'))),
    ),
    alignment: Alignment.centerLeft,
    child: _across(context, p.kids),
  ),
  Parts(lay: Lay.page) => SingleChildScrollView(
    padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 16),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [for (final k in p.kids) Padding(padding: const EdgeInsets.only(bottom: 10), child: draw(context, k))],
    ),
  ),
  Parts(lay: Lay.lines) => Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [for (final k in p.kids) draw(context, k)]),
  Parts(lay: Lay.tabs) => Row(mainAxisSize: MainAxisSize.min, children: [for (final k in p.kids) Padding(padding: const EdgeInsets.only(right: 2), child: draw(context, k))]),
  Parts() => Wrap(spacing: 6, runSpacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [for (final k in p.kids) draw(context, k)]),
  _NameBox() => _nameBox(context, p.text),
  Words() => _tip(p.tip, _words(context, p.text, p.tone)),
  Gap() => const SizedBox(width: 12),
  Act(link: true) => _tip(
    p.tip,
    InkWell(
      onTap: p.run,
      child: p.tone == Tone.plain ? Text(p.label, style: TextStyle(color: tint(context, 'link'))) : _words(context, p.label, p.tone),
    ),
  ),
  Act() => _tip(p.tip, _button(context, p)),
  Entry() => Field(p),
  Pick() => DropdownButtonHideUnderline(
    child: DropdownButton<String>(
      value: p.options.any((o) => o.$1 == p.value) ? p.value : null,
      hint: Text(p.hint),
      isDense: true,
      style: Theme.of(context).textTheme.bodyMedium,
      iconEnabledColor: Theme.of(context).textTheme.bodyMedium?.color,
      items: [for (final (value, label) in p.options) DropdownMenuItem(value: value, child: Text(label))],
      onChanged: (v) => v == null ? null : p.pick(v),
    ),
  ),
  Listing() => SingleChildScrollView(
    scrollDirection: Axis.horizontal,
    child: Table(
      defaultColumnWidth: const IntrinsicColumnWidth(),
      defaultVerticalAlignment: TableCellVerticalAlignment.middle,
      border: TableBorder(horizontalInside: BorderSide(color: tint(context, 'line')), bottom: BorderSide(color: tint(context, 'line'))),
      children: [
        if (p.head.isNotEmpty) TableRow(children: [for (final t in p.head) _listCell(Text(t, style: const TextStyle(fontWeight: FontWeight.w600)))]),
        for (final r in p.rows)
          TableRow(
            children: [
              for (final c in r.cells) _listCell(DefaultTextStyle.merge(style: TextStyle(fontWeight: r.on ? FontWeight.w600 : null), child: draw(context, c))),
              // A row may say less than the table has columns.
              for (var i = r.cells.length; i < p.head.length; i++) const SizedBox(),
            ],
          ),
      ],
    ),
  ),
  DataGrid() => Grid(p),
  SheetGrid() => _sheet(context, p),
};

/// The name box of the formula bar: a part only this surface draws.
class _NameBox extends Words {
  const _NameBox(super.text);
}

Widget _nameBox(BuildContext context, String text) => Container(
  width: 84,
  height: 26,
  padding: const EdgeInsets.symmetric(horizontal: 8),
  alignment: Alignment.centerLeft,
  decoration: BoxDecoration(border: Border.all(color: tint(context, 'line')), borderRadius: BorderRadius.circular(3)),
  child: Text(text, style: mono.copyWith(fontSize: 12)),
);

Widget _listCell(Widget child) => Padding(padding: const EdgeInsets.fromLTRB(0, 6, 18, 6), child: child);

/// Marks all of a field's text once it has the cursor, so that what is typed takes its
/// place (a field on the desktop puts the cursor at the end when it gets it).
void selectAllOnFocus(FocusNode focus, TextEditingController text) {
  void mark() {
    if (!focus.hasFocus) return;
    focus.removeListener(mark);
    WidgetsBinding.instance.addPostFrameCallback((_) => text.selection = TextSelection(baseOffset: 0, extentOffset: text.text.length));
  }

  focus.addListener(mark);
}

/// A line to type in. What is typed stays the field's own until it is done.
class Field extends StatefulWidget {
  const Field(this.entry, {this.expand = false, this.start = false, this.cancel, this.plain = false, super.key});
  final Entry entry;

  /// Takes the width it is given, not one of its own.
  final bool expand;

  /// Has the cursor from the start, with all of its text marked.
  final bool start;
  final void Function()? cancel;

  /// Without a line around it (inside a cell of a grid).
  final bool plain;

  @override
  State<Field> createState() => _FieldState();
}

class _FieldState extends State<Field> {
  late final text = TextEditingController(text: widget.entry.value);
  late final focus = FocusNode()..addListener(_left);
  bool over = false;

  @override
  void initState() {
    super.initState();
    if (widget.start) {
      focus.requestFocus();
      selectAllOnFocus(focus, text);
    }
  }

  @override
  void didUpdateWidget(Field old) {
    super.didUpdateWidget(old);
    if (old.entry.value != widget.entry.value) text.text = widget.entry.value;
  }

  @override
  void dispose() {
    focus.dispose();
    text.dispose();
    super.dispose();
  }

  void _left() {
    if (!focus.hasFocus && !widget.entry.enterOnly) _done();
  }

  void _done() {
    if (over || (text.text == widget.entry.value && !widget.start)) return;
    over = widget.start;
    widget.entry.done?.call(text.text);
  }

  @override
  Widget build(BuildContext context) {
    final e = widget.entry;
    final line = OutlineInputBorder(borderRadius: BorderRadius.circular(5), borderSide: BorderSide(color: tint(context, 'line')));
    final field = CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): () {
          over = true;
          text.text = e.value;
          widget.cancel?.call();
          over = widget.cancel != null;
        },
      },
      child: TextField(
        controller: text,
        focusNode: focus,
        enabled: e.done != null,
        style: (e.mono ? mono : const TextStyle()).copyWith(fontSize: 13),
        onSubmitted: (_) => _done(),
        decoration: InputDecoration(
          isDense: true,
          hintText: e.hint,
          hintStyle: TextStyle(color: tint(context, 'soft')),
          contentPadding: EdgeInsets.symmetric(horizontal: widget.plain ? 4 : 8, vertical: widget.plain ? 5 : 8),
          border: widget.plain ? InputBorder.none : line,
          enabledBorder: widget.plain ? InputBorder.none : line,
          disabledBorder: widget.plain ? InputBorder.none : line,
        ),
      ),
    );
    return _tip(e.tip, widget.expand ? field : SizedBox(width: e.wide ? 380 : 170, child: field));
  }
}

const _grounds = {Ground.calc: 'calc', Ground.kept: 'kept', Ground.own: 'own'};

/// A cell of a grid: a line to the right and below, its ground, its content.
Widget _box(BuildContext context, Widget child, {Color? color, Alignment align = Alignment.centerLeft, Border? frame, void Function()? tap}) {
  final line = BorderSide(color: tint(context, 'grid'));
  final box = Container(
    alignment: align,
    padding: const EdgeInsets.symmetric(horizontal: 7),
    foregroundDecoration: frame == null ? null : BoxDecoration(border: frame),
    decoration: BoxDecoration(
      color: color,
      border: Border(right: line, bottom: line),
    ),
    child: child,
  );
  return tap == null ? box : GestureDetector(behavior: HitTestBehavior.opaque, onTap: tap, child: box);
}

/// The width a column needs for texts of some lengths.
double _width(Iterable<int> lengths) => (lengths.fold(0, (a, b) => a > b ? a : b) * 7.6 + 22).clamp(70, 320).toDouble();

/// The rows of a table: names and filters stay at the top, the sums at the bottom, the
/// numbers of the rows at the left. A double click on a cell that takes a value puts a
/// field in its place.
class Grid extends StatefulWidget {
  const Grid(this.grid, {super.key});
  final DataGrid grid;

  @override
  State<Grid> createState() => _GridState();
}

class _GridState extends State<Grid> {
  (int, int)? editing;
  (int, int)? tapped;
  DateTime at = DateTime(0);

  @override
  Widget build(BuildContext context) {
    final g = widget.grid;
    final panel = tint(context, 'head'), soft = tint(context, 'soft');
    final sample = g.rows < 40 ? g.rows : 40;
    final widths = [
      for (var k = 0; k < g.cols.length; k++) _width([g.cols[k].name.length + 4, g.cols[k].type.length, g.foot[k].text.length, for (var r = 0; r < sample; r++) g.cell(r, k).text.length]),
    ];

    Widget data(int r, int k) {
      final c = g.cell(r, k);
      final color = g.head(r).picked ? tint(context, 'pick') : _grounds[c.ground] == null ? null : tint(context, _grounds[c.ground]!);
      if (editing == (r, k) && c.edit != null) {
        void end() => setState(() => editing = null);
        return _box(
          context,
          Field(
            Entry(c.raw ?? c.text, (text) {
              end();
              c.edit!(text);
            }),
            expand: true,
            start: true,
            plain: true,
            cancel: end,
          ),
          color: color,
          frame: Border.all(color: tint(context, 'accent'), width: 2),
        );
      }
      return _tip(
        c.tip,
        _box(
          context,
          Text(c.text, overflow: TextOverflow.ellipsis, style: const TextStyle(fontFeatures: [FontFeature.tabularFigures()])),
          color: color,
          align: c.number ? Alignment.centerRight : Alignment.centerLeft,
          tap: () {
            // The second click on a cell soon after the first is a double click.
            final now = DateTime.now(), twice = tapped == (r, k) && now.difference(at).inMilliseconds < 450;
            tapped = (r, k);
            at = now;
            c.tap?.call();
            if (twice && c.edit != null) setState(() => editing = (r, k));
          },
        ),
      );
    }

    Widget cell(BuildContext context, TableVicinity v) {
      final r = v.row - 2, k = v.column - 1, last = v.row == g.rows + 2;
      if (v.row == 0) {
        if (k < 0) return _box(context, Text('#', style: TextStyle(color: soft)), color: panel, align: Alignment.centerRight);
        final c = g.cols[k];
        final ground = _grounds[c.ground] == null ? panel : tint(context, _grounds[c.ground]!);
        return _tip(
          c.tip,
          _box(
            context,
            Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('${c.calc ? 'ƒ ' : ''}${c.name}${c.mark}', overflow: TextOverflow.ellipsis, style: const TextStyle(fontWeight: FontWeight.w600)),
                Text(c.type, style: TextStyle(fontSize: 10, color: soft)),
              ],
            ),
            color: ground,
            tap: () => c.sort(HardwareKeyboard.instance.isShiftPressed),
          ),
        );
      }
      if (v.row == 1) {
        final c = k < 0 ? null : g.cols[k];
        return _box(context, c?.filter == null ? const SizedBox() : Field(c!.filter!, expand: true, plain: true, key: ValueKey(c.name)), color: panel);
      }
      if (last) {
        if (k < 0) return _box(context, Text('Σ', style: TextStyle(color: soft)), color: panel, align: Alignment.centerRight);
        return _tip(g.foot[k].tip, _box(context, Text(g.foot[k].text, style: const TextStyle(fontWeight: FontWeight.w600)), color: panel, align: Alignment.centerRight));
      }
      if (k < 0) {
        final head = g.head(r);
        return _tip(head.tip, _box(context, Text(head.label, style: TextStyle(color: soft)), color: head.picked ? tint(context, 'pick') : panel, align: Alignment.centerRight, tap: head.pick));
      }
      return data(r, k);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: TableView.builder(
            columnCount: g.cols.length + 1,
            rowCount: g.rows + 3,
            pinnedRowCount: 2,
            pinnedColumnCount: 1,
            trailingPinnedRowCount: 1,
            columnBuilder: (c) => TableSpan(extent: FixedSpanExtent(c == 0 ? 52 : widths[c - 1])),
            rowBuilder: (r) => TableSpan(extent: FixedSpanExtent(r == 0 ? 38 : r == 1 ? 30 : 26)),
            cellBuilder: (context, v) => TableViewCell(child: cell(context, v)),
          ),
        ),
        if (g.more != null) Padding(padding: const EdgeInsets.all(10), child: draw(context, g.more!)),
      ],
    );
  }
}

/// A part of a sheet, cell by cell: the letters of the columns stay at the top, the
/// numbers of the rows at the left.
Widget _sheet(BuildContext context, SheetGrid g) {
  final panel = tint(context, 'head'), soft = tint(context, 'soft'), accent = tint(context, 'accent');
  Widget cell(BuildContext context, TableVicinity v) {
    final r = v.row - 1, c = v.column - 1;
    if (r < 0 || c < 0) {
      // The heads of the picked cell's row and column are marked, as in Excel.
      final at = (r < 0 && c >= 0 && g.at?.$2 == c) || (c < 0 && r >= 0 && g.at?.$1 == r);
      final mark = BorderSide(color: accent, width: 2);
      return _box(
        context,
        Text(r < 0 && c < 0 ? '' : r < 0 ? g.colName(c) : '${r + 1}', style: TextStyle(color: at ? accent : soft, fontWeight: at ? FontWeight.w600 : null)),
        color: at ? tint(context, 'pick') : panel,
        align: r < 0 ? Alignment.center : Alignment.centerRight,
        frame: at ? Border(bottom: r < 0 ? mark : BorderSide.none, right: c < 0 ? mark : BorderSide.none) : null,
      );
    }
    final cell = g.cell(r, c);
    if (cell == null) return _box(context, const SizedBox());
    final edge = cell.region == null ? null : BorderSide(color: tint(context, cell.region!.name), width: 2);
    BorderSide side(String letter) => edge != null && cell.edges.contains(letter) ? edge : BorderSide.none;
    return _tip(
      cell.tip,
      _box(
        context,
        Text(
          cell.text,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontWeight: cell.bold ? FontWeight.w600 : null, fontStyle: cell.italic ? FontStyle.italic : null, color: cell.color != null ? hex(cell.color!) : cell.fill != null ? hex(onFill) : null),
        ),
        color: cell.fill != null ? hex(cell.fill!) : cell.calc ? tint(context, 'calc') : null,
        align: switch (cell.align) {
          'right' => Alignment.centerRight,
          'center' => Alignment.center,
          _ => Alignment.centerLeft,
        },
        frame: cell.picked ? Border.all(color: accent, width: 2) : edge == null ? null : Border(top: side('t'), bottom: side('b'), left: side('l'), right: side('r')),
        tap: cell.tap,
      ),
    );
  }

  return Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Expanded(
        child: TableView.builder(
          columnCount: g.cols + 1,
          rowCount: g.rows + 1,
          pinnedRowCount: 1,
          pinnedColumnCount: 1,
          columnBuilder: (c) => TableSpan(extent: FixedSpanExtent(c == 0 ? 46 : g.width(c - 1) ?? 86)),
          rowBuilder: (r) => const TableSpan(extent: FixedSpanExtent(24)),
          cellBuilder: (context, v) {
            // A merged cell is one cell over all the places it covers (as far as loaded).
            final (r, c) = g.anchor(v.row - 1, v.column - 1) ?? (v.row - 1, v.column - 1);
            final top = r < 0 || c < 0 ? null : g.cell(r, c);
            if (top == null || (top.rowSpan == 1 && top.colSpan == 1)) return TableViewCell(child: cell(context, v));
            int span(int n, int from, int all) => from + n > all ? all - from : n;
            return TableViewCell(
              rowMergeStart: r + 1,
              rowMergeSpan: span(top.rowSpan, r, g.rows),
              columnMergeStart: c + 1,
              columnMergeSpan: span(top.colSpan, c, g.cols),
              child: cell(context, TableVicinity(row: r + 1, column: c + 1)),
            );
          },
        ),
      ),
      if (g.more != null) Padding(padding: const EdgeInsets.all(10), child: draw(context, g.more!)),
      if (g.note != null) Padding(padding: const EdgeInsets.all(10), child: _words(context, g.note!, Tone.hint)),
    ],
  );
}
