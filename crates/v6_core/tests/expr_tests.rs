use v6_core::expr::{eval_expr, parse_expression};
use v6_core::lexer::tokenize_line;

fn eval(input: &str) -> i64 {
    let tokens = tokenize_line(input, "test", 1).unwrap();
    let (expr, _) = parse_expression(&tokens).unwrap();
    eval_expr(&expr, &|_| None, 0).unwrap()
}

fn eval_with_symbols(input: &str, symbols: &[(&str, i64)]) -> i64 {
    let tokens = tokenize_line(input, "test", 1).unwrap();
    let (expr, _) = parse_expression(&tokens).unwrap();
    eval_expr(&expr, &|name| {
        symbols.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }, 0).unwrap()
}

#[test]
fn test_simple_add() {
    assert_eq!(eval("1 + 2"), 3);
}

#[test]
fn test_precedence() {
    assert_eq!(eval("2 + 3 * 4"), 14);
}

#[test]
fn test_parens() {
    assert_eq!(eval("(2 + 3) * 4"), 20);
}

#[test]
fn test_unary_minus() {
    assert_eq!(eval("-5"), -5);
}

#[test]
fn test_low_high_byte() {
    assert_eq!(eval("<$1234"), 0x34);
    assert_eq!(eval(">$1234"), 0x12);
}

#[test]
fn test_bitwise() {
    assert_eq!(eval("$FF & $0F"), 0x0F);
    assert_eq!(eval("$F0 | $0F"), 0xFF);
    assert_eq!(eval("$FF ^ $0F"), 0xF0);
}

#[test]
fn test_shifts() {
    assert_eq!(eval("1 << 4"), 16);
    assert_eq!(eval("16 >> 2"), 4);
}

#[test]
fn test_comparison() {
    assert_eq!(eval("5 > 3"), 1);
    assert_eq!(eval("3 > 5"), 0);
    assert_eq!(eval("5 == 5"), 1);
    assert_eq!(eval("5 != 5"), 0);
}

#[test]
fn test_logical() {
    assert_eq!(eval("1 && 1"), 1);
    assert_eq!(eval("1 && 0"), 0);
    assert_eq!(eval("0 || 1"), 1);
    assert_eq!(eval("!0"), 1);
    assert_eq!(eval("!1"), 0);
}

#[test]
fn test_complex_expression() {
    assert_eq!(eval("16 + 1 + 2 + 3 + 0b100_000_000 - $100 - 0x00"), 22);
}

#[test]
fn test_symbol_reference() {
    assert_eq!(eval_with_symbols("CONST1 + 5", &[("CONST1", 10)]), 15);
}

#[test]
fn test_boolean_literals() {
    assert_eq!(eval("TRUE"), 1);
    assert_eq!(eval("FALSE"), 0);
}
