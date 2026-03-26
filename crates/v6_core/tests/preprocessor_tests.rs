use v6_core::preprocessor::{
    parse_include_directive, parse_macro_args, parse_macro_params,
    replace_param, strip_multiline_comments,
};

#[test]
fn test_strip_multiline_comments() {
    let input = "before /* comment */ after";
    let result = strip_multiline_comments(input);
    assert_eq!(result, "before  after");
}

#[test]
fn test_strip_multiline_preserves_newlines() {
    let input = "line1\n/* comment\nspanning\nlines */\nline2";
    let result = strip_multiline_comments(input);
    assert_eq!(result, "line1\n\n\n\nline2");
}

#[test]
fn test_parse_include() {
    assert_eq!(parse_include_directive(".include \"test.asm\""), Some("test.asm".to_string()));
    assert_eq!(parse_include_directive(".include 'test.asm'"), Some("test.asm".to_string()));
    assert_eq!(parse_include_directive("  .include \"test.asm\"  ; comment"), Some("test.asm".to_string()));
    assert_eq!(parse_include_directive("mvi a, 0"), None);
}

#[test]
fn test_parse_macro_params() {
    let params = parse_macro_params("(n)");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "n");
    assert!(params[0].default.is_none());

    let params = parse_macro_params("(Background=$06, Border=$0e, Addr)");
    assert_eq!(params.len(), 3);
    assert_eq!(params[0].name, "Background");
    assert_eq!(params[0].default, Some("$06".to_string()));
    assert_eq!(params[2].name, "Addr");
    assert!(params[2].default.is_none());
}

#[test]
fn test_replace_param() {
    assert_eq!(replace_param("hlt n times", "n", "5"), "hlt 5 times");
    assert_eq!(replace_param("mov a, n", "n", "0x10"), "mov a, 0x10");
    assert_eq!(replace_param("innerval", "n", "5"), "innerval");
}

#[test]
fn test_parse_macro_args() {
    let args = parse_macro_args("$0b, $0f, PalettePtr");
    assert_eq!(args, vec!["$0b", "$0f", "PalettePtr"]);
}
