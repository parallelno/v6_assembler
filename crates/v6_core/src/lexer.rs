use crate::diagnostics::{AsmError, AsmResult, SourceLocation};

/// Token types produced by the lexer
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Identifier(String),
    Number(i64),
    StringLiteral(String),
    CharLiteral(char),
    /// Operators: +, -, *, /, %, <<, >>, <, <=, >, >=, ==, !=, &, ^, |, &&, ||, !, ~
    Operator(String),
    Comma,
    Colon,
    OpenParen,
    CloseParen,
    Dot,
    At,
    Newline,
    Eof,
}

impl Token {
    pub fn is_eof(&self) -> bool {
        matches!(self, Token::Eof)
    }
}

/// A token with its source location
#[derive(Debug, Clone)]
pub struct Located<T> {
    pub value: T,
    pub loc: SourceLocation,
}

impl<T> Located<T> {
    pub fn new(value: T, file: &str, line: usize, col: usize) -> Self {
        Self {
            value,
            loc: SourceLocation {
                file: file.to_string(),
                line,
                col,
            },
        }
    }
}

pub type LocatedToken = Located<Token>;

/// Tokenize a single line of assembly
pub fn tokenize_line(line: &str, file: &str, line_num: usize) -> AsmResult<Vec<LocatedToken>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Skip whitespace
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Single-line comments: ; or //
        if ch == ';' {
            break;
        }
        if ch == '/' && i + 1 < len && chars[i + 1] == '/' {
            break;
        }
        // Multi-line comment start (inline on a single line after preprocessing)
        if ch == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i < len {
                if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        let col = i + 1;

        // String literals
        if ch == '"' {
            let s = parse_string_literal(&chars, &mut i, file, line_num)?;
            tokens.push(LocatedToken::new(Token::StringLiteral(s), file, line_num, col));
            continue;
        }

        // Character literals
        if ch == '\'' {
            let c = parse_char_literal(&chars, &mut i, file, line_num)?;
            tokens.push(LocatedToken::new(Token::CharLiteral(c), file, line_num, col));
            continue;
        }

        // Numbers: $hex, 0xHex, %bin, 0bBin
        if ch == '$' && i + 1 < len && chars[i + 1].is_ascii_hexdigit() {
            let n = parse_hex(&chars, &mut i)?;
            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
            continue;
        }
        if ch == '0' && i + 1 < len && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
            let n = parse_0x_hex(&chars, &mut i)?;
            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
            continue;
        }
        if ch == '%' && i + 1 < len && (chars[i + 1] == '0' || chars[i + 1] == '1') {
            let n = parse_percent_bin(&chars, &mut i)?;
            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
            continue;
        }
        if ch == '0' && i + 1 < len && (chars[i + 1] == 'b' || chars[i + 1] == 'B')
            && i + 2 < len && (chars[i + 2] == '0' || chars[i + 2] == '1')
        {
            let n = parse_0b_bin(&chars, &mut i)?;
            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
            continue;
        }
        // Decimal numbers
        if ch.is_ascii_digit() {
            let n = parse_decimal(&chars, &mut i)?;
            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
            continue;
        }

        // b-prefix binary: bNNN where N is 0 or 1
        if (ch == 'b' || ch == 'B') && i + 1 < len && (chars[i + 1] == '0' || chars[i + 1] == '1') {
            // Check it's not part of a longer identifier
            if i == 0 || !is_ident_char(chars[i - 1]) {
                let start = i;
                i += 1; // skip 'b'
                match parse_binary_digits(&chars, &mut i) {
                    Ok(n) => {
                        // Make sure next char is not an ident char (otherwise it's an identifier)
                        if i >= len || !is_ident_char(chars[i]) {
                            tokens.push(LocatedToken::new(Token::Number(n), file, line_num, col));
                            continue;
                        }
                    }
                    Err(_) => {}
                }
                // Wasn't binary, rewind and parse as identifier
                i = start;
            }
        }

        // Two-character operators
        if i + 1 < len {
            let two = format!("{}{}", ch, chars[i + 1]);
            match two.as_str() {
                "<<" | ">>" | "<=" | ">=" | "==" | "!=" | "&&" | "||" => {
                    tokens.push(LocatedToken::new(Token::Operator(two), file, line_num, col));
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }

        // Single-character tokens
        match ch {
            '+' | '-' | '*' | '/' | '~' | '!' | '&' | '|' | '^' | '<' | '>' | '=' => {
                tokens.push(LocatedToken::new(Token::Operator(ch.to_string()), file, line_num, col));
                i += 1;
                continue;
            }
            ',' => {
                tokens.push(LocatedToken::new(Token::Comma, file, line_num, col));
                i += 1;
                continue;
            }
            ':' => {
                tokens.push(LocatedToken::new(Token::Colon, file, line_num, col));
                i += 1;
                continue;
            }
            '(' => {
                tokens.push(LocatedToken::new(Token::OpenParen, file, line_num, col));
                i += 1;
                continue;
            }
            ')' => {
                tokens.push(LocatedToken::new(Token::CloseParen, file, line_num, col));
                i += 1;
                continue;
            }
            '.' => {
                tokens.push(LocatedToken::new(Token::Dot, file, line_num, col));
                i += 1;
                continue;
            }
            '@' => {
                tokens.push(LocatedToken::new(Token::At, file, line_num, col));
                i += 1;
                continue;
            }
            '#' => {
                // # is used in some syntaxes as immediate prefix - skip it
                i += 1;
                continue;
            }
            _ => {}
        }

        // Identifiers
        if ch.is_ascii_alphabetic() || ch == '_' {
            let ident = parse_identifier(&chars, &mut i);
            tokens.push(LocatedToken::new(Token::Identifier(ident), file, line_num, col));
            continue;
        }

        return Err(AsmError::new(format!("Unexpected character: '{}'", ch))
            .with_location(SourceLocation {
                file: file.to_string(),
                line: line_num,
                col,
            }));
    }

    Ok(tokens)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn parse_identifier(chars: &[char], i: &mut usize) -> String {
    let start = *i;
    while *i < chars.len() && is_ident_char(chars[*i]) {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

fn parse_decimal(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    let start = *i;
    while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '_') {
        *i += 1;
    }
    let s: String = chars[start..*i].iter().filter(|c| **c != '_').collect();
    s.parse::<i64>().map_err(|_| AsmError::new(format!("Invalid decimal number: {}", s)))
}

fn parse_hex(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    *i += 1; // skip '$'
    let start = *i;
    while *i < chars.len() && (chars[*i].is_ascii_hexdigit() || chars[*i] == '_') {
        *i += 1;
    }
    let s: String = chars[start..*i].iter().filter(|c| **c != '_').collect();
    i64::from_str_radix(&s, 16).map_err(|_| AsmError::new(format!("Invalid hex number: ${}", s)))
}

fn parse_0x_hex(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    *i += 2; // skip '0x'
    let start = *i;
    while *i < chars.len() && (chars[*i].is_ascii_hexdigit() || chars[*i] == '_') {
        *i += 1;
    }
    let s: String = chars[start..*i].iter().filter(|c| **c != '_').collect();
    i64::from_str_radix(&s, 16).map_err(|_| AsmError::new(format!("Invalid hex number: 0x{}", s)))
}

fn parse_percent_bin(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    *i += 1; // skip '%'
    parse_binary_digits(chars, i)
}

fn parse_0b_bin(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    *i += 2; // skip '0b'
    parse_binary_digits(chars, i)
}

fn parse_binary_digits(chars: &[char], i: &mut usize) -> AsmResult<i64> {
    let start = *i;
    while *i < chars.len() && (chars[*i] == '0' || chars[*i] == '1' || chars[*i] == '_') {
        *i += 1;
    }
    let s: String = chars[start..*i].iter().filter(|c| **c != '_').collect();
    if s.is_empty() {
        return Err(AsmError::new("Empty binary literal"));
    }
    i64::from_str_radix(&s, 2).map_err(|_| AsmError::new(format!("Invalid binary number: {}", s)))
}

fn parse_escape(chars: &[char], i: &mut usize) -> AsmResult<char> {
    if *i >= chars.len() {
        return Err(AsmError::new("Unexpected end of escape sequence"));
    }
    let ch = chars[*i];
    *i += 1;
    match ch {
        'n' => Ok('\n'),
        't' => Ok('\t'),
        'r' => Ok('\r'),
        '0' => Ok('\0'),
        '\\' => Ok('\\'),
        '"' => Ok('"'),
        '\'' => Ok('\''),
        _ => Err(AsmError::new(format!("Unknown escape sequence: \\{}", ch))),
    }
}

fn parse_string_literal(chars: &[char], i: &mut usize, file: &str, line_num: usize) -> AsmResult<String> {
    let col = *i + 1;
    *i += 1; // skip opening "
    let mut s = String::new();
    while *i < chars.len() && chars[*i] != '"' {
        if chars[*i] == '\\' {
            *i += 1;
            s.push(parse_escape(chars, i)?);
        } else {
            s.push(chars[*i]);
            *i += 1;
        }
    }
    if *i >= chars.len() {
        return Err(AsmError::new("Unterminated string literal")
            .with_location(SourceLocation {
                file: file.to_string(),
                line: line_num,
                col,
            }));
    }
    *i += 1; // skip closing "
    Ok(s)
}

fn parse_char_literal(chars: &[char], i: &mut usize, file: &str, line_num: usize) -> AsmResult<char> {
    let col = *i + 1;
    *i += 1; // skip opening '
    let ch = if *i < chars.len() && chars[*i] == '\\' {
        *i += 1;
        parse_escape(chars, i)?
    } else if *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        c
    } else {
        return Err(AsmError::new("Empty character literal")
            .with_location(SourceLocation {
                file: file.to_string(),
                line: line_num,
                col,
            }));
    };
    if *i >= chars.len() || chars[*i] != '\'' {
        return Err(AsmError::new("Unterminated character literal")
            .with_location(SourceLocation {
                file: file.to_string(),
                line: line_num,
                col,
            }));
    }
    *i += 1; // skip closing '
    Ok(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decimal() {
        let tokens = tokenize_line("42", "test", 1).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, Token::Number(42));
    }

    #[test]
    fn test_hex_dollar() {
        let tokens = tokenize_line("$FF", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(255));
    }

    #[test]
    fn test_hex_0x() {
        let tokens = tokenize_line("0xFF", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(255));
    }

    #[test]
    fn test_binary_percent() {
        let tokens = tokenize_line("%1010", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(10));
    }

    #[test]
    fn test_binary_0b() {
        let tokens = tokenize_line("0b1010", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(10));
    }

    #[test]
    fn test_binary_b_prefix() {
        let tokens = tokenize_line("b1010", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(10));
    }

    #[test]
    fn test_binary_with_underscores() {
        let tokens = tokenize_line("%11_00", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(12));
    }

    #[test]
    fn test_char_literal() {
        let tokens = tokenize_line("'A'", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::CharLiteral('A'));
    }

    #[test]
    fn test_char_escape() {
        let tokens = tokenize_line("'\\n'", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::CharLiteral('\n'));
    }

    #[test]
    fn test_string_literal() {
        let tokens = tokenize_line("\"hello\\nworld\"", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::StringLiteral("hello\nworld".to_string()));
    }

    #[test]
    fn test_instruction_line() {
        let tokens = tokenize_line("  mvi a, 0x10  ; comment", "test", 1).unwrap();
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].value, Token::Identifier("mvi".to_string()));
        assert_eq!(tokens[1].value, Token::Identifier("a".to_string()));
        assert_eq!(tokens[2].value, Token::Comma);
        assert_eq!(tokens[3].value, Token::Number(16));
    }

    #[test]
    fn test_operators() {
        let tokens = tokenize_line("<< >> <= >= == != && ||", "test", 1).unwrap();
        assert_eq!(tokens.len(), 8);
        assert_eq!(tokens[0].value, Token::Operator("<<".to_string()));
        assert_eq!(tokens[1].value, Token::Operator(">>".to_string()));
    }

    #[test]
    fn test_label_with_colon() {
        let tokens = tokenize_line("start:", "test", 1).unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].value, Token::Identifier("start".to_string()));
        assert_eq!(tokens[1].value, Token::Colon);
    }

    #[test]
    fn test_local_label() {
        let tokens = tokenize_line("@loop:", "test", 1).unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].value, Token::At);
        assert_eq!(tokens[1].value, Token::Identifier("loop".to_string()));
        assert_eq!(tokens[2].value, Token::Colon);
    }

    #[test]
    fn test_directive() {
        let tokens = tokenize_line(".org 0x100", "test", 1).unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].value, Token::Dot);
        assert_eq!(tokens[1].value, Token::Identifier("org".to_string()));
        assert_eq!(tokens[2].value, Token::Number(256));
    }

    #[test]
    fn test_b_prefix_binary() {
        let tokens = tokenize_line("b00_011_111", "test", 1).unwrap();
        assert_eq!(tokens[0].value, Token::Number(0b00_011_111));
    }

    #[test]
    fn test_inline_multiline_comment() {
        let tokens = tokenize_line("mvi a, 5 /* comment */ mvi b, 6", "test", 1).unwrap();
        assert_eq!(tokens.len(), 8);
    }
}
