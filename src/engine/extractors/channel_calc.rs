//! Channel-calculation extractor — tiny expression DSL over per-pixel
//! color channels.
//!
//! Lets the user write short formulas like `max(0, R - G)` or `1 - L` and
//! have them evaluated once per pixel to build a density map. This is
//! the escape hatch when none of the other extractors do exactly what's
//! needed — useful for custom black plates, hue isolation, duotone
//! intermediates, and user experimentation.
//!
//! Supported grammar (recursive descent):
//!
//! ```text
//! expr    := term (('+' | '-') term)*
//! term    := factor (('*' | '/') factor)*
//! factor  := '-' factor | primary
//! primary := number
//!          | identifier
//!          | identifier '(' expr (',' expr)* ')'
//!          | '(' expr ')'
//! ```
//!
//! Identifiers:
//!
//! | Name | Value range | Meaning                                     |
//! |------|-------------|---------------------------------------------|
//! | `R`  | `[0, 1]`    | sRGB red channel                             |
//! | `G`  | `[0, 1]`    | sRGB green channel                           |
//! | `B`  | `[0, 1]`    | sRGB blue channel                            |
//! | `L`  | `[0, 1]`    | LAB lightness rescaled from [0, 100]         |
//! | `a`  | `[-1, 1]`   | LAB a, rescaled from [-128, 127]             |
//! | `b`  | `[-1, 1]`   | LAB b, rescaled from [-128, 127]             |
//! | `K`  | `[0, 1]`    | GCR black: `1 - max(R, G, B)`                |
//!
//! Built-in functions: `max(a, b)`, `min(a, b)`, `abs(x)`, `invert(x)`
//! (shorthand for `1 - x`), `clip(x, lo, hi)`.
//!
//! The expression is parsed once up front and the resulting AST is
//! walked per pixel. The AST nodes are tiny (`Box<Expr>`) so this is
//! still fast; if it ever becomes a hot spot we'll compile to a flat
//! bytecode vector.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::{rgb_to_lab, Rgb};

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Expr {
    Number(f32),
    Var(Var),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum Var {
    R,
    G,
    B,
    L,
    A,
    Bb,
    K,
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    Neg,
}

/// Built-in function tokens. Public because it appears in
/// [`CalcError::BadArity`] which is part of the public error surface.
#[derive(Debug, Clone, Copy)]
pub enum Func {
    Max,
    Min,
    Abs,
    Invert,
    Clip,
}

#[derive(Debug, thiserror::Error)]
pub enum CalcError {
    #[error("unexpected character at position {0}: {1:?}")]
    UnexpectedChar(usize, char),
    #[error("unexpected end of expression")]
    UnexpectedEnd,
    #[error("unknown identifier: {0}")]
    UnknownIdent(String),
    #[error("function {0:?} expects {1} args, got {2}")]
    BadArity(Func, usize, usize),
    #[error("expected '{0}' at position {1}")]
    Expected(char, usize),
}

// ---------------------------------------------------------------------------
// Lexer + parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f32),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
}

fn lex(src: &str) -> Result<Vec<(usize, Token)>, CalcError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        match c {
            '(' => {
                out.push((start, Token::LParen));
                i += 1;
            }
            ')' => {
                out.push((start, Token::RParen));
                i += 1;
            }
            ',' => {
                out.push((start, Token::Comma));
                i += 1;
            }
            '+' => {
                out.push((start, Token::Plus));
                i += 1;
            }
            '-' => {
                out.push((start, Token::Minus));
                i += 1;
            }
            '*' => {
                out.push((start, Token::Star));
                i += 1;
            }
            '/' => {
                out.push((start, Token::Slash));
                i += 1;
            }
            d if d.is_ascii_digit() || d == '.' => {
                let mut j = i;
                while j < bytes.len() && ((bytes[j] as char).is_ascii_digit() || bytes[j] == b'.') {
                    j += 1;
                }
                let num: f32 = src[i..j]
                    .parse()
                    .map_err(|_| CalcError::UnexpectedChar(start, c))?;
                out.push((start, Token::Number(num)));
                i = j;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut j = i;
                while j < bytes.len() {
                    let cj = bytes[j] as char;
                    if cj.is_ascii_alphanumeric() || cj == '_' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                out.push((start, Token::Ident(src[i..j].to_string())));
                i = j;
            }
            _ => return Err(CalcError::UnexpectedChar(start, c)),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(_, t)| t)
    }
    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t.map(|(_, t)| t)
    }
    fn cur_pos(&self) -> usize {
        self.tokens.get(self.pos).map(|(p, _)| *p).unwrap_or(0)
    }

    fn parse_expr(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.parse_term()?;
        while let Some(tok) = self.peek() {
            let op = match tok {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_term()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.parse_factor()?;
        while let Some(tok) = self.peek() {
            let op = match tok {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let right = self.parse_factor()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, CalcError> {
        if let Some(Token::Minus) = self.peek() {
            self.advance();
            let inner = self.parse_factor()?;
            return Ok(Expr::Unary(UnaryOp::Neg, Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, CalcError> {
        let pos = self.cur_pos();
        match self.advance().ok_or(CalcError::UnexpectedEnd)? {
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::LParen => {
                let e = self.parse_expr()?;
                match self.advance() {
                    Some(Token::RParen) => Ok(e),
                    _ => Err(CalcError::Expected(')', pos)),
                }
            }
            Token::Ident(name) => {
                if let Some(Token::LParen) = self.peek() {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        args.push(self.parse_expr()?);
                        while matches!(self.peek(), Some(Token::Comma)) {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    match self.advance() {
                        Some(Token::RParen) => {}
                        _ => return Err(CalcError::Expected(')', pos)),
                    }
                    let func = resolve_func(&name)?;
                    expect_arity(func, args.len())?;
                    Ok(Expr::Call(func, args))
                } else {
                    Ok(Expr::Var(resolve_var(&name)?))
                }
            }
            other => Err(CalcError::UnexpectedChar(pos, debug_token(&other))),
        }
    }
}

fn debug_token(t: &Token) -> char {
    match t {
        Token::LParen => '(',
        Token::RParen => ')',
        Token::Comma => ',',
        Token::Plus => '+',
        Token::Minus => '-',
        Token::Star => '*',
        Token::Slash => '/',
        _ => '?',
    }
}

fn resolve_var(name: &str) -> Result<Var, CalcError> {
    Ok(match name {
        "R" => Var::R,
        "G" => Var::G,
        "B" => Var::B,
        "L" => Var::L,
        "a" => Var::A,
        "b" => Var::Bb,
        "K" => Var::K,
        other => return Err(CalcError::UnknownIdent(other.to_string())),
    })
}

fn resolve_func(name: &str) -> Result<Func, CalcError> {
    Ok(match name {
        "max" => Func::Max,
        "min" => Func::Min,
        "abs" => Func::Abs,
        "invert" => Func::Invert,
        "clip" => Func::Clip,
        other => return Err(CalcError::UnknownIdent(other.to_string())),
    })
}

fn expect_arity(func: Func, n: usize) -> Result<(), CalcError> {
    let want = match func {
        Func::Max | Func::Min => 2,
        Func::Abs | Func::Invert => 1,
        Func::Clip => 3,
    };
    if n != want {
        Err(CalcError::BadArity(func, want, n))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Channels {
    r: f32,
    g: f32,
    b: f32,
    l: f32,
    a: f32,
    bb: f32,
    k: f32,
}

fn eval(expr: &Expr, ch: &Channels) -> f32 {
    match expr {
        Expr::Number(n) => *n,
        Expr::Var(v) => match v {
            Var::R => ch.r,
            Var::G => ch.g,
            Var::B => ch.b,
            Var::L => ch.l,
            Var::A => ch.a,
            Var::Bb => ch.bb,
            Var::K => ch.k,
        },
        Expr::Unary(UnaryOp::Neg, inner) => -eval(inner, ch),
        Expr::Binary(op, l, r) => {
            let lv = eval(l, ch);
            let rv = eval(r, ch);
            match op {
                BinOp::Add => lv + rv,
                BinOp::Sub => lv - rv,
                BinOp::Mul => lv * rv,
                BinOp::Div => {
                    if rv.abs() < 1e-9 {
                        0.0
                    } else {
                        lv / rv
                    }
                }
            }
        }
        Expr::Call(func, args) => match func {
            Func::Max => eval(&args[0], ch).max(eval(&args[1], ch)),
            Func::Min => eval(&args[0], ch).min(eval(&args[1], ch)),
            Func::Abs => eval(&args[0], ch).abs(),
            Func::Invert => 1.0 - eval(&args[0], ch),
            Func::Clip => {
                let x = eval(&args[0], ch);
                let lo = eval(&args[1], ch);
                let hi = eval(&args[2], ch);
                x.clamp(lo, hi)
            }
        },
    }
}

/// Parse an expression once and return a reusable compiled form.
pub struct Compiled(Expr);

pub fn parse(src: &str) -> Result<Compiled, CalcError> {
    let tokens = lex(src)?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_expr()?;
    Ok(Compiled(expr))
}

/// Evaluate a parsed expression over every pixel of `source`. The return
/// value of the expression is clamped to `[0, 1]` and interpreted as ink
/// coverage (1 = full ink).
pub fn extract(source: &RgbImage, expr: &Compiled) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for (x, y, p) in source.enumerate_pixels() {
        let r = p[0] as f32 / 255.0;
        let g = p[1] as f32 / 255.0;
        let b = p[2] as f32 / 255.0;
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
        let l = (lab.l / 100.0).clamp(0.0, 1.0);
        let a = (lab.a / 128.0).clamp(-1.0, 1.0);
        let bb = (lab.b / 128.0).clamp(-1.0, 1.0);
        let k = 1.0 - r.max(g).max(b);
        let ch = Channels {
            r,
            g,
            b,
            l,
            a,
            bb,
            k,
        };
        let v = eval(&expr.0, &ch).clamp(0.0, 1.0);
        let density = ((1.0 - v) * 255.0).round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

/// Convenience: parse-and-run in one call. Useful for one-shot GUI
/// previews where the expression might not be valid yet.
pub fn extract_expr(source: &RgbImage, src: &str) -> Result<GrayImage, CalcError> {
    let compiled = parse(src)?;
    Ok(extract(source, &compiled))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(src: &RgbImage, x: u32, y: u32, expr: &str) -> u8 {
        let compiled = parse(expr).unwrap();
        let out = extract(src, &compiled);
        out.get_pixel(x, y)[0]
    }

    #[test]
    fn simple_expression() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        // R = 1 → full ink → density 0
        assert_eq!(pixel(&src, 0, 0, "R"), 0);
        // 1 - R = 0 → no ink
        assert_eq!(pixel(&src, 0, 0, "1 - R"), 255);
    }

    #[test]
    fn max_and_abs() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([200, 100, 50]));
        // max(0, R - G) ≈ 0.39 → density ≈ 155
        let v = pixel(&src, 0, 0, "max(0, R - G)");
        assert!((150..=160).contains(&(v as i32)));
    }

    #[test]
    fn k_channel_matches_gcr() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([30, 30, 30]));
        // K = 1 - max = 1 - 30/255 ≈ 0.88 → density ≈ 30
        let v = pixel(&src, 0, 0, "K");
        assert!((25..=35).contains(&(v as i32)));
    }

    #[test]
    fn unknown_identifier_errors() {
        assert!(parse("Q + 1").is_err());
    }

    #[test]
    fn arity_mismatch_errors() {
        assert!(parse("max(R)").is_err());
    }
}
