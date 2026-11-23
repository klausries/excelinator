/// The screens of the app as HTML: each kind of part is drawn here once. The classes
/// are those of `web/app.css`.
library;

import 'package:jaspr/jaspr.dart';
import 'package:web/web.dart' as web;
import 'package:xl_app_core/xl_app_core.dart';

import 'host.dart';

/// An element: its tag with classes (`td.num.calc`), attributes (null and false are
/// left out), events, and children (components, texts; null is left out).
Component h(String tag, [Map<String, Object?> attrs = const {}, List<Object?> kids = const [], Map<String, EventCallback>? on, Key? key]) {
  final [name, ...classes] = tag.split('.');
  return Component.element(
    tag: name,
    key: key,
    classes: [...classes, attrs['class']].where((c) => c != null && c != '').join(' ').nullIfEmpty,
    attributes: {
      for (final e in attrs.entries)
        if (e.key != 'class' && e.value != null && e.value != false) e.key: e.value == true ? '' : '${e.value}',
    },
    events: on,
    children: [
      for (final k in kids)
        if (k is Component) k else if (k != null) Component.text('$k'),
    ],
  );
}

extension on String {
  String? get nullIfEmpty => isEmpty ? null : this;
}

Map<String, EventCallback> click(void Function()? run) => {
  if (run != null)
    'click': (e) {
      e.preventDefault();
      run();
    },
};

/// The app on the page: head, side, content, and what was said.
class Surface extends StatefulComponent {
  const Surface(this.app, this.host, {super.key});
  final App app;
  final PageHost host;

  @override
  State<Surface> createState() => _SurfaceState();
}

class _SurfaceState extends State<Surface> {
  @override
  void initState() {
    super.initState();
    component.app.listen(() => setState(() {}));
    component.host.changed = () => setState(() {});
  }

  @override
  Component build(BuildContext context) {
    final s = component.app.screen;
    web.document.body?.classList.toggle('busy', s.busy);
    // The head in two lines, as Excel has it: the title bar (the app, the store), then
    // the levels as tabs with what can be done on the right.
    final tabs = s.head.indexWhere((p) => p is Parts && p.lay == Lay.tabs);
    final title = tabs < 0 ? s.head : s.head.take(tabs);
    final ribbon = tabs < 0 ? const <Part>[] : s.head.skip(tabs);
    final last = s.messages.lastOrNull;
    return Component.fragment([
      h('header', {'id': 'head'}, [
        h('div.title', {}, title.map(draw).toList()),
        if (ribbon.isNotEmpty) h('div.ribbon', {}, ribbon.map(draw).toList()),
      ]),
      h('div.body', {}, [
        h('nav', {'id': 'side'}, [for (final p in s.side) p is Act ? _item(p) : draw(p)]),
        h('main', {'id': 'main'}, [
          ...s.main.map(draw),
          if (s.foot.isNotEmpty)
            h('div.sheettabs', {'id': 'sheets'}, [for (final a in s.foot) h('a.tab', {'class': a.on ? 'on' : '', 'href': a.href ?? '#', 'title': a.tip}, [a.label], click(a.run))]),
        ]),
      ]),
      h('footer', {'id': 'status'}, [
        h('span', {'class': last?.error == true ? 'error' : ''}, [s.busy ? 'Arbeitet …' : last?.text ?? 'Bereit']),
      ]),
      h('div', {'id': 'toasts'}, [for (final m in s.messages) h('div.toast', {'class': m.error ? 'error' : ''}, [m.text])]),
      if (component.host.question case final q?) _dialog(q, component.host.answer),
    ]);
  }
}

/// A question to the person, over the page: a text to type, or yes or no.
Component _dialog(Question q, void Function(String?) answer) => h('div.modal', {}, [
  h('div.dialog', {}, [
    h('p', {}, [q.text]),
    if (q.input) Field(Entry(q.initial, answer, enterOnly: true), start: true, cancel: () => answer(null), key: ValueKey(q)),
    h('div.buttons', {}, [
      h('button', {}, ['Abbrechen'], click(() => answer(null))),
      h('button.on', {}, ['OK'], click(() => answer(q.input ? (web.document.querySelector('.dialog input') as web.HTMLInputElement).value : ''))),
    ]),
  ]),
]);

/// A table or sheet in the list at the side.
Component _item(Act a) => h('a.item', {'class': a.on ? 'on' : '', 'href': a.href ?? '#', 'title': a.tip}, [h('span', {}, [a.label]), if (a.note != null) h('small', {}, [a.note])], click(a.run));

const _lays = {Lay.bar: 'div.bar', Lay.formula: 'div.formula', Lay.page: 'div.pad', Lay.lines: 'div.lines', Lay.line: 'span.line', Lay.tabs: 'span.tabs'};

const _tones = {
  Tone.plain: 'span', Tone.brand: 'b.brand', Tone.title: 'h2', Tone.heading: 'h3', Tone.strong: 'b', Tone.soft: 'small', Tone.hint: 'span.hint', Tone.warn: 'small.warn',
  Tone.badgeWarn: 'span.badge.warn', Tone.badgeOk: 'span.badge.ok', Tone.mono: 'code', Tone.report: 'pre', Tone.read: 'span.read',
  Tone.table: 'span.chip.table', Tone.cells: 'span.chip.cells', Tone.block: 'span.chip.block',
};

Component draw(Part p) => switch (p) {
  // On a page, plain words are a paragraph.
  Parts(lay: Lay.page) => h(_lays[Lay.page]!, {}, [for (final k in p.kids) k is Words && k.tone == Tone.plain ? h('p', {}, [k.text]) : draw(k)]),
  Parts() => h(_lays[p.lay]!, {}, p.kids.map(draw).toList()),
  Words() => h(_tones[p.tone]!, {'title': p.tip}, [p.text]),
  Gap() => h('span.grow'),
  Act(link: true) => h('a', {'class': _tones[p.tone]!.split('.').skip(1).join(' '), 'href': p.href ?? '#', 'title': p.tip}, [p.label], click(p.run)),
  Act() => h('button', {'class': p.on ? 'on' : '', 'disabled': p.run == null, 'title': p.tip}, [p.label], click(p.run)),
  Entry() => Field(p),
  Pick() => h(
    'select',
    {'title': p.hint},
    [
      h('option', {'value': '', 'disabled': true, 'selected': p.value == null}, [p.hint]),
      for (final (value, label) in p.options) h('option', {'value': value, 'selected': value == p.value}, [label]),
    ],
    {'change': (e) => p.pick((e.target as web.HTMLSelectElement).value)},
  ),
  Listing() => h('table.plain', {'class': p.wide ? 'wide' : ''}, [
    if (p.head.isNotEmpty) h('tr', {}, [for (final t in p.head) h('th', {}, [t])]),
    for (final r in p.rows) h('tr', {'class': r.on ? 'on' : ''}, [for (final c in r.cells) h('td', {'class': c is Words && RegExp(r'^\d+$').hasMatch(c.text) ? 'num' : ''}, [draw(c)])]),
  ]),
  DataGrid() => Grid(p),
  SheetGrid() => _sheet(p),
};

/// A line to type in. What is typed stays the field's own until it is done, whatever
/// is drawn anew meanwhile.
class Field extends StatefulComponent {
  const Field(this.entry, {this.classes = '', this.start = false, this.cancel, super.key});
  final Entry entry;
  final String classes;

  /// Has the cursor from the start, with all of its text marked.
  final bool start;
  final void Function()? cancel;

  @override
  State<Field> createState() => _FieldState();
}

class _FieldState extends State<Field> {
  String? typed;
  final id = 'f${identityHashCode(Object())}';

  @override
  void initState() {
    super.initState();
    if (component.start) {
      Future.microtask(() {
        final e = web.document.getElementById(id) as web.HTMLInputElement?;
        e?.focus();
        e?.select();
      });
    }
  }

  @override
  void didUpdateComponent(Field old) {
    super.didUpdateComponent(old);
    if (old.entry.value != component.entry.value) typed = null;
  }

  /// A field that came with the cursor is done once: Enter or Escape end it, and its
  /// leaving the page then (a blur) is no second word.
  bool over = false;

  void done() {
    if (over) return;
    final text = typed;
    typed = null;
    over = component.start;
    if (text != null || component.start) component.entry.done?.call(text ?? component.entry.value);
  }

  @override
  Component build(BuildContext context) {
    final e = component.entry;
    return h(
      'input',
      {'id': id, 'class': '${component.classes} ${e.mono ? 'mono' : ''} ${e.wide ? 'wide' : ''}', 'value': typed ?? e.value, 'placeholder': e.hint, 'title': e.tip, 'disabled': e.done == null},
      const [],
      {
        'input': (ev) => typed = (ev.target as web.HTMLInputElement).value,
        'keydown': (ev) {
          final key = (ev as web.KeyboardEvent).key;
          if (key == 'Enter') done();
          if (key == 'Escape') {
            typed = null;
            over = component.start;
            component.cancel != null ? component.cancel!() : setState(() {});
          }
        },
        if (!e.enterOnly) 'blur': (_) => typed != null || component.start ? done() : null,
      },
    );
  }
}

/// The rows of a table. A double click on a cell that takes a value puts a field in
/// its place.
class Grid extends StatefulComponent {
  const Grid(this.grid, {super.key});
  final DataGrid grid;

  @override
  State<Grid> createState() => _GridState();
}

const _grounds = {Ground.plain: '', Ground.calc: 'calc', Ground.kept: 'kept', Ground.own: 'own'};

class _GridState extends State<Grid> {
  /// The cell that is typed in: (row, column).
  (int, int)? editing;

  @override
  Component build(BuildContext context) {
    final g = component.grid;
    Component cell(int r, int k) {
      final c = g.cell(r, k);
      final attrs = {'class': '${c.number ? 'num' : ''} ${_grounds[c.ground]}', 'title': c.tip, 'data-at': '$r,$k'};
      if (editing == (r, k) && c.edit != null) {
        void end() => setState(() => editing = null);
        final field = Field(Entry(c.raw ?? c.text, (text) {
          end();
          c.edit!(text);
        }), classes: 'edit', start: true, cancel: end);
        return h('td', attrs, [field]);
      }
      return h('td', attrs, [c.text], {
        'click': (_) => c.tap?.call(),
        if (c.edit != null) 'dblclick': (_) => setState(() => editing = (r, k)),
      });
    }

    return h('div.grid', {'id': 'grid'}, [
      h('table', {}, [
        h('thead', {}, [
          h('tr', {}, [
            h('th.corner', {}, ['#']),
            for (final c in g.cols)
              h('th', {'class': _grounds[c.ground], 'title': c.tip}, [h('span', {}, ['${c.calc ? 'ƒ ' : ''}${c.name}${c.mark}']), h('small', {}, [c.type])], {'click': (e) => c.sort((e as web.MouseEvent).shiftKey)}),
          ]),
          h('tr.filters', {}, [
            h('th'),
            for (final c in g.cols)
              h('th', {}, [
                if (c.filter != null) Field(c.filter!, classes: 'filter', key: ValueKey(c.name)),
              ]),
          ]),
        ]),
        h('tbody', {}, [
          for (var r = 0; r < g.rows; r++)
            () {
              final head = g.head(r);
              return h('tr', {'class': head.picked ? 'picked' : ''}, [
                h('th', {'title': head.tip}, [head.label], click(head.pick)),
                for (var k = 0; k < g.cols.length; k++) cell(r, k),
              ]);
            }(),
        ]),
        h('tfoot', {}, [
          h('tr', {}, [
            h('th', {}, ['Σ']),
            for (final c in g.foot) h('td.num', {'title': c.tip}, [c.text]),
          ]),
        ]),
      ]),
      if (g.more != null) draw(g.more!),
    ]);
  }
}

/// A part of a sheet, cell by cell.
Component _sheet(SheetGrid g) {
  String? style(SheetCell c) => [
    if (c.bold) 'font-weight:600',
    if (c.italic) 'font-style:italic',
    if (c.fill != null) 'background:${c.fill}',
    if (c.color != null) 'color:${c.color}' else if (c.fill != null) 'color:$onFill',
    if (c.align != null) 'text-align:${c.align}',
  ].join(';').nullIfEmpty;
  return h('div.grid.sheet', {'id': 'grid'}, [
    h('table', {}, [
      h('thead', {}, [
        h('tr', {}, [
          h('th.corner'),
          for (var c = 0; c < g.cols; c++) h('th', {'class': g.at?.$2 == c ? 'at' : '', 'style': g.width(c) == null ? null : 'min-width:${g.width(c)}px'}, [g.colName(c)]),
        ]),
      ]),
      h('tbody', {}, [
        for (var r = 0; r < g.rows; r++)
          h('tr', {}, [
            h('th', {'class': g.at?.$1 == r ? 'at' : ''}, ['${r + 1}']),
            for (var c = 0; c < g.cols; c++)
              if (g.cell(r, c) case final cell?)
                h(
                  'td',
                  {
                    'class': '${cell.region == null ? '' : 'in ${cell.region!.name}'} ${cell.edges.isEmpty ? '' : 'edge-${cell.edges}'} ${cell.calc ? 'calc' : ''} ${cell.picked ? 'picked' : ''}',
                    'style': style(cell),
                    'rowspan': cell.rowSpan > 1 ? cell.rowSpan : null,
                    'colspan': cell.colSpan > 1 ? cell.colSpan : null,
                    'title': cell.tip,
                    'data-at': '$r,$c',
                  },
                  [cell.text],
                  click(cell.tap),
                ),
          ]),
      ]),
    ]),
    if (g.more != null) draw(g.more!),
    if (g.note != null) h('p.hint', {}, [g.note]),
  ]);
}
