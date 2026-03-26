use crate::diagnostics::{AsmError, AsmResult};
use crate::lexer::{LocatedToken, Token};

/// Expression AST node
#[derive(Debug, Clone)]
pub enum Expr {
    Number(i64),
    Symbol(String),
    LocalSymbol(String),
    CurrentPC,
    BoolLiteral(bool),
    UnaryOp { op: UnaryOp, expr: Box<Expr> },
    BinaryOp { op: BinaryOp, left: Box<Expr>, right: Box<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    BitNot,
    LowByte,
    HighByte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    LogAnd,
    LogOr,
}

impl BinaryOp {
    fn precedence(self) -> u8 {
        match self {
            BinaryOp::LogOr => 1,
            BinaryOp::LogAnd => 2,
            BinaryOp::BitOr => 3,
            BinaryOp::BitXor => 4,
            BinaryOp::BitAnd => 5,
            BinaryOp::Eq | BinaryOp::Ne => 6,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 7,
            BinaryOp::Shl | BinaryOp::Shr => 8,
            BinaryOp::Add | BinaryOp::Sub => 9,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 10,
        }
    }
}

/// Evaluate a parsed expression given a symbol resolver.
/// Returns Err if a symbol cannot be resolved.
pub fn eval_expr(
    expr: &Expr,
    resolve_symbol: &dyn Fn(&str) -> Option<i64>,
    pc: u16,
) -> AsmResult<i64> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::BoolLiteral(b) => Ok(if *b { 1 } else { 0 }),
        Expr::CurrentPC => Ok(pc as i64),
        Expr::Symbol(name) => {
            resolve_symbol(name).ok_or_else(|| AsmError::new(format!("Undefined symbol: {}", name)))
        }
        Expr::LocalSymbol(name) => {
            resolve_symbol(name).ok_or_else(|| AsmError::new(format!("Undefined local symbol: @{}", name)))
        }
        Expr::UnaryOp { op, expr } => {
            let val = eval_expr(expr, resolve_symbol, pc)?;
            Ok(match op {
                UnaryOp::Plus => val,
                UnaryOp::Minus => -val,
                UnaryOp::Not => if val == 0 { 1 } else { 0 },
                UnaryOp::BitNot => !val,
                UnaryOp::LowByte => val & 0xFF,
                UnaryOp::HighByte => (val >> 8) & 0xFF,
            })
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval_expr(left, resolve_symbol, pc)?;
            let r = eval_expr(right, resolve_symbol, pc)?;
            Ok(match op {
                BinaryOp::Add => l.wrapping_add(r),
                BinaryOp::Sub => l.wrapping_sub(r),
                BinaryOp::Mul => l.wrapping_mul(r),
                BinaryOp::Div => {
                    if r == 0 {
                        return Err(AsmError::new("Division by zero"));
                    }
                    l / r
                }
                BinaryOp::Mod => {
                    if r == 0 {
                        return Err(AsmError::new("Modulo by zero"));
                    }
                    l % r
                }
                BinaryOp::Shl => l.wrapping_shl(r as u32),
                BinaryOp::Shr => l.wrapping_shr(r as u32),
                BinaryOp::Lt => if l < r { 1 } else { 0 },
                BinaryOp::Le => if l <= r { 1 } else { 0 },
                BinaryOp::Gt => if l > r { 1 } else { 0 },
                BinaryOp::Ge => if l >= r { 1 } else { 0 },
                BinaryOp::Eq => if l == r { 1 } else { 0 },
                BinaryOp::Ne => if l != r { 1 } else { 0 },
                BinaryOp::BitAnd => l & r,
                BinaryOp::BitXor => l ^ r,
                BinaryOp::BitOr => l | r,
                BinaryOp::LogAnd => if l != 0 && r != 0 { 1 } else { 0 },
                BinaryOp::LogOr => if l != 0 || r != 0 { 1 } else { 0 },
            })
        }
    }
}

/// Expression parser using Pratt parsing / recursive descent
pub struct ExprParser<'a> {
    tokens: &'a [LocatedToken],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    pub fn new(tokens: &'a [LocatedToken]) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.value)
    }

    fn advance(&mut self) -> Option<&LocatedToken> {
        if self.pos < self.tokens.len() {
            let t = &self.tokens[self.pos];
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    /// Parse a full expression
    pub fn parse_expr(&mut self) -> AsmResult<Expr> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> AsmResult<Expr> {
        let mut left = self.parse_unary()?;

        loop {
            let op = match self.peek_binary_op() {
                Some(op) if op.precedence() >= min_prec => op,
                _ => break,
            };

            self.advance(); // consume operator
            let right = self.parse_binary(op.precedence() + 1)?;
            left = Expr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn peek_binary_op(&self) -> Option<BinaryOp> {
        match self.peek()? {
            Token::Operator(s) => match s.as_str() {
                "+" => Some(BinaryOp::Add),
                "-" => Some(BinaryOp::Sub),
                "*" => Some(BinaryOp::Mul),
                "/" => Some(BinaryOp::Div),
                "%" => Some(BinaryOp::Mod),
                "<<" => Some(BinaryOp::Shl),
                ">>" => Some(BinaryOp::Shr),
                "<" => Some(BinaryOp::Lt),
                "<=" => Some(BinaryOp::Le),
                ">" => Some(BinaryOp::Gt),
                ">=" => Some(BinaryOp::Ge),
                "==" => Some(BinaryOp::Eq),
                "!=" => Some(BinaryOp::Ne),
                "&" => Some(BinaryOp::BitAnd),
                "^" => Some(BinaryOp::BitXor),
                "|" => Some(BinaryOp::BitOr),
                "&&" => Some(BinaryOp::LogAnd),
                "||" => Some(BinaryOp::LogOr),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_unary(&mut self) -> AsmResult<Expr> {
        match self.peek() {
            Some(Token::Operator(s)) => {
                let op = match s.as_str() {
                    "+" => Some(UnaryOp::Plus),
                    "-" => Some(UnaryOp::Minus),
                    "!" => Some(UnaryOp::Not),
                    "~" => Some(UnaryOp::BitNot),
                    "<" => Some(UnaryOp::LowByte),
                    ">" => Some(UnaryOp::HighByte),
                    _ => None,
                };
                if let Some(op) = op {
                    self.advance();
                    let expr = self.parse_unary()?;
                    return Ok(Expr::UnaryOp {
                        op,
                        expr: Box::new(expr),
                    });
                }
                self.parse_primary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> AsmResult<Expr> {
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Some(Token::CharLiteral(c)) => {
                self.advance();
                Ok(Expr::Number(c as i64))
            }
            Some(Token::Operator(ref s)) if s == "*" => {
                self.advance();
                Ok(Expr::CurrentPC)
            }
            Some(Token::OpenParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                match self.peek() {
                    Some(Token::CloseParen) => {
                        self.advance();
                        Ok(expr)
                    }
                    _ => Err(AsmError::new("Expected closing parenthesis")),
                }
            }
            Some(Token::At) => {
                self.advance();
                match self.peek().cloned() {
                    Some(Token::Identifier(name)) => {
                        self.advance();
                        Ok(Expr::LocalSymbol(name))
                    }
                    _ => Err(AsmError::new("Expected identifier after @")),
                }
            }
            Some(Token::Identifier(ref name)) => {
                let upper = name.to_uppercase();
                match upper.as_str() {
                    "TRUE" => {
                        self.advance();
                        Ok(Expr::BoolLiteral(true))
                    }
                    "FALSE" => {
                        self.advance();
                        Ok(Expr::BoolLiteral(false))
                    }
                    _ => {
                        let name = name.clone();
                        self.advance();
                        Ok(Expr::Symbol(name))
                    }
                }
            }
            Some(Token::StringLiteral(_)) => {
                // StringLiteral in an expression context - shouldn't happen normally
                Err(AsmError::new("String literals not allowed in expressions"))
            }
            _ => Err(AsmError::new("Expected expression")),
        }
    }
}

/// Convenience function to parse an expression from a token slice
pub fn parse_expression(tokens: &[LocatedToken]) -> AsmResult<(Expr, usize)> {
    let mut parser = ExprParser::new(tokens);
    let expr = parser.parse_expr()?;
    Ok((expr, parser.pos()))
}
