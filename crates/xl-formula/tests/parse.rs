use xl_formula::{parse, BinaryOp, Expr, RefKind};

/// Fully parenthesized rendering that shows the tree shape.
fn shape(e: &Expr) -> String {
    match e {
        Expr::Binary { op, lhs, rhs, .. } => format!("({} {:?} {})", shape(lhs), op, shape(rhs)),
        Expr::Unary { op, expr } => format!("({op}{})", shape(expr)),
        Expr::Postfix { expr, op } => format!("({}{op})", shape(expr)),
        Expr::Paren(e) => format!("[{}]", shape(e)),
        Expr::Spaced { expr, .. } => shape(expr),
        Expr::Call { name, args } => format!("{name}<{}>", args.iter().map(shape).collect::<Vec<_>>().join("|")),
        Expr::Missing => "_".into(),
        other => other.to_string(),
    }
}

fn s(f: &str) -> String {
    shape(&parse(f).unwrap_or_else(|e| panic!("{f}: {e}")))
}

#[test]
fn precedence_and_associativity() {
    assert_eq!(s("1+2*3"), "(1 Add (2 Mul 3))");
    assert_eq!(s("1-2-3"), "((1 Sub 2) Sub 3)");
    assert_eq!(s("2^3^2"), "((2 Pow 3) Pow 2)"); // left associative, unlike mathematics
    assert_eq!(s("-2^2"), "((-2) Pow 2)"); // unary minus binds tighter than ^
    assert_eq!(s("2^-2"), "(2 Pow (-2))");
    assert_eq!(s("-A1:B2"), "(-(A1 Range B2))");
    assert_eq!(s("1+2&3"), "((1 Add 2) Concat 3)");
    assert_eq!(s("1&2=3"), "((1 Concat 2) Eq 3)");
    assert_eq!(s("50%*2"), "((50%) Mul 2)");
    assert_eq!(s("1<=2<>3"), "((1 Le 2) Ne 3)");
    assert_eq!(s("--A1"), "(-(-A1))");
}

#[test]
fn reference_operators() {
    assert_eq!(s("A1:B2 B1:C3"), "((A1 Range B2) Intersect (B1 Range C3))");
    assert_eq!(s("SUM((A1,B2))"), "SUM<[(A1 Union B2)]>");
    assert_eq!(s("SUM(A1,B2)"), "SUM<A1|B2>");
    assert_eq!(s("A1:INDEX(B:B,3)"), "(A1 Range INDEX<B:B|3>)");
    assert_eq!(s("Sheet1!A1,Sheet1!B2"), "(Sheet1!A1 Union Sheet1!B2)");
}

#[test]
fn atoms() {
    use Expr::*;
    let p = |f: &str| parse(f).unwrap();
    assert_eq!(p("$A$1"), Ref { prefix: None, kind: RefKind::Cell, text: "$A$1".into() });
    assert_eq!(p("XFD1048576"), Ref { prefix: None, kind: RefKind::Cell, text: "XFD1048576".into() });
    assert_eq!(p("XFE1"), Name { prefix: None, text: "XFE1".into() }); // beyond the last column
    assert_eq!(p("A1048577"), Name { prefix: None, text: "A1048577".into() });
    assert_eq!(p("$A:$C"), Ref { prefix: None, kind: RefKind::Columns, text: "$A:$C".into() });
    assert_eq!(p("1:3"), Ref { prefix: None, kind: RefKind::Rows, text: "1:3".into() });
    assert_eq!(p("'My Sheet'!B2"), Ref { prefix: Some("'My Sheet'!".into()), kind: RefKind::Cell, text: "B2".into() });
    assert_eq!(p("'It''s'!B2"), Ref { prefix: Some("'It''s'!".into()), kind: RefKind::Cell, text: "B2".into() });
    assert_eq!(p("[1]Sheet1!A1"), Ref { prefix: Some("[1]Sheet1!".into()), kind: RefKind::Cell, text: "A1".into() });
    assert_eq!(p("Sheet1:Sheet3!A1"), Ref { prefix: Some("Sheet1:Sheet3!".into()), kind: RefKind::Cell, text: "A1".into() });
    assert_eq!(p("Sheet1!#REF!"), Error("Sheet1!#REF!".into()));
    assert_eq!(p("#N/A"), Error("#N/A".into()));
    assert_eq!(p("TRUE"), Bool("TRUE".into()));
    assert_eq!(p("1.5E+10"), Number("1.5E+10".into()));
    assert_eq!(p(".5"), Number(".5".into()));
    assert_eq!(p("\"a\"\"b\""), Str("\"a\"\"b\"".into()));
    assert_eq!(p("Tax_Rate"), Name { prefix: None, text: "Tax_Rate".into() });
    assert_eq!(p("LOG10(100)"), Call { name: "LOG10".into(), args: vec![Number("100".into())] });
    assert_eq!(p("TRUE()"), Call { name: "TRUE".into(), args: vec![] });
    assert_eq!(p("Table1[[#This Row],[Net amount]]"), Structured { table: Some("Table1".into()), spec: "[[#This Row],[Net amount]]".into() });
    assert_eq!(p("[@Amount]"), Structured { table: None, spec: "[@Amount]".into() });
    assert_eq!(p("A1#"), Postfix { expr: Box::new(p("A1")), op: '#' });
}

#[test]
fn calls_and_arrays() {
    assert_eq!(s("IF(A1,,2)"), "IF<A1|_|2>");
    assert_eq!(s("IF(A1,)"), "IF<A1|_>");
    assert_eq!(s("PI()"), "PI<>");
    assert_eq!(parse("{1,2;3,4}").unwrap(), Expr::Array(vec![
        vec![Expr::Number("1".into()), Expr::Number("2".into())],
        vec![Expr::Number("3".into()), Expr::Number("4".into())],
    ]));
    assert_eq!(parse("SUM(_xlfn.XLOOKUP(1,A:A,B:B),max(1,2))").unwrap().functions(), ["SUM", "_XLFN.XLOOKUP", "MAX"]);
    assert!(matches!(parse("A1 B1").unwrap(), Expr::Binary { op: BinaryOp::Intersect, .. }));
}

#[test]
fn text_round_trips_exactly() {
    for f in [
        "1+2", " 1 + 2 ", "SUM( A1 , B2 )", "IF(A1>0,\"yes\",\"no\")", "A1:B2 B1:C3", "SUM((A1,B2))",
        "{1,2;3,4}", "{ \"a\" , -1 ; TRUE , #N/A }", "-A1%", "'My Sheet'!$A$1:$B$2", "[1]Sheet1!A1",
        "Table1[[#This Row],[Net amount]]*1.19", "SUM(Table1[Amount])", "IF(A1,,2)", "IF( , ,)", "PI( )",
        "LAMBDA(x,x+1)(5)", "A1#", "@A1:A10", "Sheet1:Sheet3!A1", "1.50E+03", "TRUE", "true", "#DIV/0!",
        "SUM(1:1,A:A)", "IF(A1=1,\n  2,\n  3)", "A1&\" \"\"x\"\" \"", "Übersicht!A1+Straße", "_xlfn.XLOOKUP(1,A:A,B:B)",
        "SUM(Sheet1!A1:Sheet1!B2)", "Sheet1!#REF!", "3%%", "(1)", "((A1))",
        "BT98*0607Local!$H$14", "2024!A1+1", "Sheet\u{30fb}1!$A$1:$A$9", "['file:///tmp/a b.xlsx']Tabelle1!A1", "[1]!MyName", "1.5:2", "{1,,2;,,}", "SUM($1:$3)", "Sheet1!$2:5", "{\"a\",#N/A;,}",
    ] {
        let e = parse(f).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(e.to_string(), f);
    }
}

#[test]
fn rejects_invalid_input() {
    for f in ["", "1+", "(1", "SUM(1", "\"abc", "1 2 +", "{1,2", "#FOO!", "A1)", "'Sheet'A1", "Table1[Col"] {
        assert!(parse(f).is_err(), "{f} should not parse");
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let f = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(parse(&f).is_err());
}
