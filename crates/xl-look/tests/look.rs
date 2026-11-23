//! The layers of a cell's look, each with what Excel's model says it must give.

use xl_cells::{CondFormat, CondRule, NamedStyle, Row, Column, Sheet, Style, Value, Workbook};
use xl_look::color::tinted;
use xl_look::{table, Looker, Source, Val};

fn rgb(s: &str) -> Val {
    Val::Rgb(s.into())
}

fn book() -> Workbook {
    let mut wb = Workbook { sheets: vec![Sheet { name: "Data".into(), ..Default::default() }], ..Default::default() };
    wb.styles = vec![Style::default()];
    wb
}

/// Through a file: what is written is read back and resolved.
fn reread(wb: &Workbook) -> Workbook {
    xl_cells::read_bytes(xl_cells::write_bytes(wb).unwrap()).unwrap()
}

#[test]
fn an_empty_cell_has_the_default_look() {
    let wb = reread(&book());
    let look = Looker::new(&wb).look(0, 5, 5);
    assert_eq!(look.value("font_name"), Some(&Val::Text("Calibri".into())));
    assert_eq!(look.value("font_size"), Some(&Val::Number(11.0)));
    // The default font is in the theme's text colour (dark 1).
    assert_eq!(look.value("font_color"), Some(&rgb("000000")));
    assert_eq!(look.get("font_color").unwrap().spec.as_deref(), Some("theme 1"));
    assert_eq!(look.value("fill_pattern"), Some(&Val::Text("none".into())));
    assert_eq!(look.value("fill_color"), Some(&Val::None));
    assert_eq!(look.value("locked"), Some(&Val::Flag(true)));
    assert!(look.attrs.iter().all(|(_, a)| a.source == Source::Default && a.certain));
}

#[test]
fn named_styles_and_own_formats_are_told_apart() {
    let mut wb = book();
    let good = Style { fill: Some("FFC6EFCE".into()), font_color: Some("FF006100".into()), ..Style::default() };
    wb.named_styles = vec![
        NamedStyle { name: "Normal".into(), style: Style::default(), builtin_id: Some(0), ..Default::default() },
        NamedStyle { name: "Good".into(), style: good.clone(), builtin_id: Some(26), ..Default::default() },
    ];
    let as_good = wb.intern_style(&Style { named: Some("Good".into()), ..good.clone() });
    let good_bold_red = wb.intern_style(&Style { named: Some("Good".into()), bold: true, fill: Some("FFFF0000".into()), ..good.clone() });
    wb.cells.push(0, 0, 0, Value::Number(1.0), None, as_good);
    wb.cells.push(0, 1, 0, Value::Number(2.0), None, good_bold_red);
    let wb = reread(&wb);
    let mut l = Looker::new(&wb);
    let a = l.look(0, 0, 0);
    assert_eq!(a.value("fill_color"), Some(&rgb("C6EFCE")));
    assert_eq!(a.get("fill_color").unwrap().source, Source::Style("Good".into()));
    assert_eq!(a.get("font_color").unwrap().source, Source::Style("Good".into()));
    assert_eq!(a.get("bold").unwrap().source, Source::Default);
    let b = l.look(0, 1, 0);
    assert_eq!(b.value("fill_color"), Some(&rgb("FF0000")));
    assert_eq!(b.get("fill_color").unwrap().source, Source::Cell);
    assert_eq!(b.get("bold").unwrap().source, Source::Cell);
    assert_eq!(b.get("font_color").unwrap().source, Source::Style("Good".into()));
}

#[test]
fn rows_and_columns_format_cells_without_an_entry() {
    let mut wb = book();
    let yellow = wb.intern_style(&Style { fill: Some("FFFFFF00".into()), ..Style::default() });
    let italic = wb.intern_style(&Style { italic: true, ..Style::default() });
    wb.sheets[0].rows.push(Row { row: 2, style: Some(yellow), custom_format: true, ..Row::default() });
    wb.sheets[0].columns.push(Column { first: 3, last: 4, style: Some(italic), ..Column::width(3, 4, 9.0) });
    wb.cells.push(0, 2, 0, Value::Number(1.0), None, 0);
    let wb = reread(&wb);
    let mut l = Looker::new(&wb);
    let row_cell = l.look(0, 2, 7);
    assert_eq!(row_cell.value("fill_color"), Some(&rgb("FFFF00")));
    assert_eq!(row_cell.get("fill_color").unwrap().source, Source::Row(2));
    // The row's format wins over the column's.
    assert_eq!(l.look(0, 2, 3).value("italic"), Some(&Val::Flag(false)));
    let col_cell = l.look(0, 9, 4);
    assert_eq!(col_cell.value("italic"), Some(&Val::Flag(true)));
    assert_eq!(col_cell.get("italic").unwrap().source, Source::Column(4));
    // A cell with an entry of its own has its own format, the row's notwithstanding.
    assert_eq!(l.look(0, 2, 0).value("fill_color"), Some(&Val::None));
}

#[test]
fn every_built_in_table_style_is_complete() {
    let presets = table::presets();
    let tables: Vec<_> = presets.keys().filter(|k| k.starts_with("TableStyle")).collect();
    assert_eq!(tables.len(), 60);
    assert_eq!(presets.len(), 144);
    for (name, elements) in presets {
        assert!(!elements.is_empty(), "{name}");
        for e in elements {
            assert!(!e.look.is_empty(), "{name} {} has no format", e.kind);
        }
    }
}

fn table_book(style: &str, stripes: bool) -> Workbook {
    let mut wb = book();
    for r in 0..6 {
        for c in 0..3 {
            let v = if r == 0 { Value::Text(format!("H{c}")) } else { Value::Number((r * 10 + c) as f64) };
            wb.cells.push(0, r, c, v, None, 0);
        }
    }
    wb.sheets[0].tables.push(format!(
        "<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"1\" name=\"T\" displayName=\"T\" ref=\"A1:C6\" totalsRowShown=\"0\"><autoFilter ref=\"A1:C6\"/><tableColumns count=\"3\"><tableColumn id=\"1\" name=\"H0\"/><tableColumn id=\"2\" name=\"H1\"/><tableColumn id=\"3\" name=\"H2\"/></tableColumns><tableStyleInfo name=\"{style}\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"{}\" showColumnStripes=\"0\"/></table>",
        u8::from(stripes)
    ));
    wb
}

#[test]
fn a_table_style_colours_header_and_stripes() {
    // TableStyleMedium9 (ECMA-376): header bold in light 1 on accent 1 with a thick
    // light 1 bottom border; first row stripe accent 1 at 60 % tint; the whole table
    // accent 1 at 80 % tint with thin inner borders in light 1, text in dark 1.
    let wb = reread(&table_book("TableStyleMedium9", true));
    let mut l = Looker::new(&wb);
    let accent1 = "4472C4";
    let h = l.look(0, 0, 1);
    assert_eq!(h.value("fill_color"), Some(&rgb(accent1)));
    assert_eq!(h.value("font_color"), Some(&rgb("FFFFFF")));
    assert_eq!(h.value("bold"), Some(&Val::Flag(true)));
    assert_eq!(h.value("border_bottom"), Some(&Val::Text("thick".into())));
    assert_eq!(h.get("fill_color").unwrap().source, Source::Table { table: "T".into(), style: "TableStyleMedium9".into(), element: "headerRow".into() });
    let first = l.look(0, 1, 1);
    assert_eq!(first.value("fill_color"), Some(&rgb(&tinted(accent1, 0.59999389629810485))));
    assert_eq!(first.get("fill_color").unwrap().source, Source::Table { table: "T".into(), style: "TableStyleMedium9".into(), element: "firstRowStripe".into() });
    let second = l.look(0, 2, 1);
    assert_eq!(second.value("fill_color"), Some(&rgb(&tinted(accent1, 0.79998168889431442))));
    // Inner borders between cells of the table, none on its outer edge (wholeTable
    // gives none there).
    assert_eq!(second.value("border_left"), Some(&Val::Text("thin".into())));
    assert_eq!(second.value("border_left_color"), Some(&rgb("FFFFFF")));
    assert_eq!(l.look(0, 2, 0).value("border_left"), Some(&Val::Text("none".into())));
    // Without row stripes every data row has the whole table's look.
    let wb = reread(&table_book("TableStyleMedium9", false));
    assert_eq!(Looker::new(&wb).look(0, 1, 1).value("fill_color"), Some(&rgb(&tinted(accent1, 0.79998168889431442))));
    // Outside the table nothing of it.
    assert_eq!(l.look(0, 7, 1).value("fill_color"), Some(&Val::None));
}

#[test]
fn a_cells_own_format_wins_over_the_table_style() {
    let mut wb = table_book("TableStyleMedium9", true);
    let red = wb.intern_style(&Style { fill: Some("FFFF0000".into()), ..Style::default() });
    let i = (0..wb.cells.len()).find(|&i| wb.cells.row[i] == 1 && wb.cells.col[i] == 1).unwrap();
    wb.cells.style[i] = red;
    let wb = reread(&wb);
    let mut l = Looker::new(&wb);
    let a = l.look(0, 1, 1);
    assert_eq!(a.value("fill_color"), Some(&rgb("FF0000")));
    assert_eq!(a.get("fill_color").unwrap().source, Source::Cell);
    // The font colour the cell leaves at the default comes from the table.
    assert!(matches!(a.get("font_color").unwrap().source, Source::Table { .. }));
}

#[test]
fn an_unknown_table_style_gives_nothing() {
    let wb = reread(&table_book("NoSuchStyle", true));
    assert_eq!(Looker::new(&wb).look(0, 0, 0).value("fill_color"), Some(&Val::None));
}

fn cf_book(rules: Vec<(&str, CondRule)>) -> Workbook {
    let mut wb = book();
    for r in 0..10 {
        wb.cells.push(0, r, 0, Value::Number((r + 1) as f64), None, 0);
        wb.cells.push(0, r, 1, Value::Text(if r % 2 == 0 { "apple".into() } else { "pear".into() }), None, 0);
    }
    for (sqref, rule) in rules {
        wb.sheets[0].cond_formats.push(CondFormat { sqref: sqref.into(), rules: vec![rule] });
    }
    wb
}

fn rule(attrs: &[(&str, &str)], inner: &str, dxf: Option<&str>) -> CondRule {
    CondRule { attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), inner: inner.into(), dxf: dxf.map(str::to_string) }
}

const RED_FILL: &str = "<fill><patternFill><bgColor rgb=\"FFFF0000\"/></patternFill></fill>";
const GREEN_TEXT: &str = "<font><b/><color rgb=\"FF00B050\"/></font>";

#[test]
fn formula_rules_are_filled_to_each_cell() {
    let wb = reread(&cf_book(vec![
        ("A1:A10", rule(&[("type", "cellIs"), ("priority", "2"), ("operator", "greaterThan")], "<formula>5</formula>", Some(RED_FILL))),
        ("B1:B10", rule(&[("type", "expression"), ("priority", "1")], "<formula>AND($A1&gt;2,B1=\"apple\")</formula>", Some(GREEN_TEXT))),
    ]));
    let mut l = Looker::new(&wb);
    let cells: Vec<(u32, u32, u32)> = (0..10).flat_map(|r| [(0, r, 0), (0, r, 1)]).collect();
    let looks = l.looks(&cells);
    for r in 0..10u32 {
        let a = &looks[(2 * r) as usize];
        let want = if r + 1 > 5 { rgb("FF0000") } else { Val::None };
        assert_eq!(a.value("fill_color"), Some(&want), "A{}", r + 1);
        if r + 1 > 5 {
            assert_eq!(a.get("fill_color").unwrap().source, Source::Conditional { range: "A1:A10".into(), priority: 2, kind: "cellIs".into() });
        }
        let b = &looks[(2 * r + 1) as usize];
        let hit = r + 1 > 2 && r % 2 == 0;
        assert_eq!(b.value("bold"), Some(&Val::Flag(hit)), "B{}", r + 1);
        assert_eq!(b.value("font_color"), Some(&rgb(if hit { "00B050" } else { "000000" })));
        assert!(b.undecided.is_empty());
    }
}

#[test]
fn priority_and_stop_if_true() {
    let wb = reread(&cf_book(vec![
        ("A1:A10", rule(&[("type", "cellIs"), ("priority", "1"), ("operator", "between"), ("stopIfTrue", "1")], "<formula>3</formula><formula>4</formula>", Some(GREEN_TEXT))),
        ("A1:A10", rule(&[("type", "cellIs"), ("priority", "2"), ("operator", "greaterThan")], "<formula>2</formula>", Some(RED_FILL))),
    ]));
    let mut l = Looker::new(&wb);
    let looks = l.looks(&[(0, 1, 0), (0, 2, 0), (0, 4, 0)]);
    // 2: neither. 3: the first rule, which stops the second. 5: only the second.
    assert_eq!(looks[0].value("fill_color"), Some(&Val::None));
    assert_eq!(looks[1].value("bold"), Some(&Val::Flag(true)));
    assert_eq!(looks[1].value("fill_color"), Some(&Val::None));
    assert_eq!(looks[2].value("bold"), Some(&Val::Flag(false)));
    assert_eq!(looks[2].value("fill_color"), Some(&rgb("FF0000")));
}

#[test]
fn an_undecidable_rule_marks_what_it_would_set() {
    let wb = reread(&cf_book(vec![("A1:A10", rule(&[("type", "expression"), ("priority", "1")], "<formula>NOSUCHFUNCTION(A1)</formula>", Some(RED_FILL)))]));
    let look = Looker::new(&wb).look(0, 0, 0);
    assert!(!look.get("fill_color").unwrap().certain);
    assert!(look.get("bold").unwrap().certain);
    assert_eq!(look.undecided.len(), 1);
}

#[test]
fn range_rules() {
    let wb = reread(&cf_book(vec![
        ("A1:A10", rule(&[("type", "colorScale"), ("priority", "1")], "<colorScale><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF0000FF\"/><color rgb=\"FFFFFFFF\"/></colorScale>", None)),
        ("A1:A10", rule(&[("type", "top10"), ("priority", "2"), ("rank", "3")], "", Some(GREEN_TEXT))),
        ("B1:B10", rule(&[("type", "duplicateValues"), ("priority", "3")], "", Some(GREEN_TEXT))),
        ("A1:A10", rule(&[("type", "iconSet"), ("priority", "4")], "<iconSet iconSet=\"3Arrows\"><cfvo type=\"percent\" val=\"0\"/><cfvo type=\"percent\" val=\"33\"/><cfvo type=\"percent\" val=\"67\"/></iconSet>", None)),
        ("A1:A10", rule(&[("type", "dataBar"), ("priority", "5")], "<dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar>", None)),
        ("A1:A10", rule(&[("type", "aboveAverage"), ("priority", "6")], "", Some("<font><i/></font>"))),
    ]));
    let mut l = Looker::new(&wb);
    let a: Vec<_> = l.looks(&(0..10).map(|r| (0, r, 0)).collect::<Vec<_>>());
    // Colour scale from blue (1) to white (10), linear per channel: as LibreOffice's
    // HTML export gives it for the same rule.
    let want = ["0000FF", "1C1CFF", "3838FF", "5555FF", "7171FF", "8D8DFF", "AAAAFF", "C6C6FF", "E2E2FF", "FFFFFF"];
    for (r, w) in want.iter().enumerate() {
        assert_eq!(a[r].value("fill_color"), Some(&rgb(w)), "A{}", r + 1);
    }
    // Top 3: 8, 9, 10.
    assert_eq!(a.iter().map(|x| x.value("bold") == Some(&Val::Flag(true))).collect::<Vec<_>>(), (1..=10).map(|v| v >= 8).collect::<Vec<_>>());
    // Above the average 5.5.
    assert_eq!(a.iter().map(|x| x.value("italic") == Some(&Val::Flag(true))).collect::<Vec<_>>(), (1..=10).map(|v| v > 5).collect::<Vec<_>>());
    // Icons: thresholds at 1 + 9·33 % and 1 + 9·67 %.
    assert_eq!(a.iter().map(|x| x.icon.as_ref().unwrap().index).collect::<Vec<_>>(), vec![0, 0, 0, 1, 1, 1, 1, 2, 2, 2]);
    assert_eq!(a[0].data_bar.as_ref().unwrap().fraction, 0.0);
    assert_eq!(a[9].data_bar.as_ref().unwrap().fraction, 1.0);
    assert_eq!(a[9].data_bar.as_ref().unwrap().color.as_deref(), Some("638EC6"));
    // "apple" and "pear" each occur five times.
    assert_eq!(l.look(0, 3, 1).value("bold"), Some(&Val::Flag(true)));
}

#[test]
fn rules_know_their_cell_and_the_moment() {
    let wb = reread(&cf_book(vec![
        ("A1:A10", rule(&[("type", "expression"), ("priority", "1")], "<formula>MOD(ROW(),2)=0</formula>", Some(GREEN_TEXT))),
        ("B1:B10", rule(&[("type", "expression"), ("priority", "2")], "<formula>$A1=DAY(TODAY())</formula>", Some(RED_FILL))),
        ("A1:A10", rule(&[("type", "expression"), ("priority", "3")], "<formula>1/0</formula>", Some("<font><i/></font>"))),
    ]));
    let mut l = Looker::new(&wb);
    // 2026-11-04 (serial 46330) at noon.
    l.now = 46330.5;
    let a: Vec<_> = l.looks(&(0..10).map(|r| (0, r, 0)).collect::<Vec<_>>());
    assert_eq!(a.iter().map(|x| x.value("bold") == Some(&Val::Flag(true))).collect::<Vec<_>>(), (1..=10).map(|r| r % 2 == 0).collect::<Vec<_>>());
    // A rule whose formula is an error applies nothing, and is decided.
    assert!(a.iter().all(|x| x.value("italic") == Some(&Val::Flag(false)) && x.undecided.is_empty()));
    let b: Vec<_> = l.looks(&(0..10).map(|r| (0, r, 1)).collect::<Vec<_>>());
    assert_eq!(b.iter().map(|x| x.value("fill_color") == Some(&rgb("FF0000"))).collect::<Vec<_>>(), (1..=10).map(|v| v == 4).collect::<Vec<_>>());
}

#[test]
fn an_undecided_rule_above_leaves_what_a_rule_below_sets_uncertain() {
    let blue = "<fill><patternFill><bgColor rgb=\"FF0000FF\"/></patternFill></fill>";
    let wb = reread(&cf_book(vec![
        ("A1:A10", rule(&[("type", "expression"), ("priority", "1")], "<formula>NOSUCHFUNCTION(A1)</formula>", Some(RED_FILL))),
        ("A1:A10", rule(&[("type", "cellIs"), ("priority", "2"), ("operator", "greaterThan")], "<formula>0</formula>", Some(blue))),
    ]));
    let look = Looker::new(&wb).look(0, 0, 0);
    // Blue as far as is known; red if the first rule holds.
    assert_eq!(look.value("fill_color"), Some(&rgb("0000FF")));
    assert!(!look.get("fill_color").unwrap().certain);
}

#[test]
fn a_palette_entry_that_is_no_colour_is_not_taken() {
    use xl_look::color::{Colors, Spec, DEFAULT_PALETTE};
    let colors = Colors::new(None, &["FFF".to_string(), "80ÿ0000".to_string(), "FF123456".to_string()]);
    assert_eq!(colors.rgb(&Spec::Indexed(0, 0.0), None).as_deref(), Some(DEFAULT_PALETTE[0]));
    assert_eq!(colors.rgb(&Spec::Indexed(1, 0.4), None), Some(tinted(DEFAULT_PALETTE[1], 0.4)));
    assert_eq!(colors.rgb(&Spec::Indexed(2, 0.0), None).as_deref(), Some("123456"));
    // Text that is no colour does not stop the work.
    tinted("FFF", 0.4);
    tinted("ÿÿÿ", -0.4);
}

#[test]
fn formulas_are_written_for_the_top_left_of_the_whole_range() {
    // As Excel reads them (checked with `ROW(A1)=1` rules): the corner of the bounding box
    // of all areas, wherever they stand in `sqref`, and even where it is no cell of the range.
    let fill = |cell: (u32, u32), sqref: &str, formula: &str| {
        let wb = reread(&cf_book(vec![(sqref, rule(&[("type", "expression"), ("priority", "1")], formula, Some(RED_FILL)))]));
        let look = Looker::new(&wb).look(0, cell.0, cell.1);
        assert!(look.undecided.is_empty());
        look.value("fill_color") == Some(&rgb("FF0000"))
    };
    let here = "<formula>AND(ROW(A1)=1,COLUMN(A1)=1)</formula>";
    assert!(fill((0, 0), "C5:C10 A1:A3", here) && !fill((4, 2), "C5:C10 A1:A3", here));
    // The corner A1 is no cell of `A5:A10 C1:C3`: only a rule on the row shows it.
    let row = "<formula>ROW(A1)=1</formula>";
    for sqref in ["A5:A10 C1:C3", "C1:C3 A5:A10"] {
        assert!(!fill((4, 0), sqref, row) && fill((0, 2), sqref, row), "{sqref}");
        assert!(!fill((0, 2), sqref, here), "{sqref}");
    }
}

#[test]
fn another_moment_decides_again() {
    let wb = reread(&cf_book(vec![("B1:B10", rule(&[("type", "expression"), ("priority", "1")], "<formula>$A1=DAY(TODAY())</formula>", Some(RED_FILL)))]));
    let mut l = Looker::new(&wb);
    let cells: Vec<_> = (0..10).map(|r| (0, r, 1)).collect();
    let red = |l: &mut Looker| l.looks(&cells).iter().position(|x| x.value("fill_color") == Some(&rgb("FF0000")));
    // 2026-11-04 at noon, then the day after.
    l.now = 46330.5;
    assert_eq!(red(&mut l), Some(3));
    l.now = 46331.5;
    assert_eq!(red(&mut l), Some(4));
}

#[test]
fn rules_of_a_type_are_decided_by_their_type() {
    // As Excel does: an error value is "not blank", although the rule's formula is an error there.
    let mut wb = cf_book(vec![
        ("C1:C3", rule(&[("type", "notContainsBlanks"), ("priority", "1")], "<formula>LEN(TRIM(C1))&gt;0</formula>", Some("<font><i/></font>"))),
        ("D1:D10", rule(&[("type", "timePeriod"), ("priority", "2"), ("timePeriod", "last7Days")], "<formula>NOSUCHFUNCTION(D1)</formula>", Some(RED_FILL))),
        ("D1:D10", rule(&[("type", "timePeriod"), ("priority", "3"), ("timePeriod", "thisWeek")], "<formula>NOSUCHFUNCTION(D1)</formula>", Some(GREEN_TEXT))),
    ]);
    wb.cells.push(0, 0, 2, Value::Error("#DIV/0!".into()), None, 0);
    wb.cells.push(0, 1, 2, Value::Text("  ".into()), None, 0);
    // 2026-11-04 (a Wednesday) is serial 46330; the days around it.
    for r in 0..10u32 {
        wb.cells.push(0, r, 3, Value::Number(46323.0 + r as f64), None, 0);
    }
    let wb = reread(&wb);
    let mut l = Looker::new(&wb);
    l.now = 46330.5;
    let c: Vec<_> = l.looks(&[(0, 0, 2), (0, 1, 2), (0, 2, 2)]);
    assert_eq!(c.iter().map(|x| x.value("italic") == Some(&Val::Flag(true))).collect::<Vec<_>>(), vec![true, false, false]);
    let d: Vec<_> = l.looks(&(0..10).map(|r| (0, r, 3)).collect::<Vec<_>>());
    assert!(d.iter().all(|x| x.undecided.is_empty() && x.get("fill_color").unwrap().certain));
    // Last 7 days: 46324 to 46330. This week: Sunday 46327 to Saturday 46333.
    assert_eq!(d.iter().map(|x| x.value("fill_color") == Some(&rgb("FF0000"))).collect::<Vec<_>>(), (46323..46333).map(|s| (46324..=46330).contains(&s)).collect::<Vec<_>>());
    assert_eq!(d.iter().map(|x| x.value("bold") == Some(&Val::Flag(true))).collect::<Vec<_>>(), (46323..46333).map(|s| (46327..=46333).contains(&s)).collect::<Vec<_>>());
}
