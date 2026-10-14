//! Folder rules: boolean expressions over tags.
//!
//! ```text
//! rule := or
//! or   := and ("OR" and)*
//! and  := not ("AND" not)*
//! not  := "NOT" not | atom
//! atom := TAG | "(" or ")"
//! ```

use std::collections::HashSet;
use std::fmt;

use crate::tags::has_tag;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Matches the tag and everything below it.
    Tag(String),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
}

impl Expr {
    pub fn parse(input: &str) -> Result<Expr, String> {
        let tokens = tokenize(input);
        if tokens.is_empty() {
            return Err("empty rule".to_string());
        }
        let mut pos = 0;
        let expr = parse_or(&tokens, &mut pos)?;
        if pos < tokens.len() {
            return Err(format!("unexpected {} after a complete rule", tokens[pos]));
        }
        Ok(expr)
    }

    pub fn matches(&self, tags: &HashSet<String>) -> bool {
        match self {
            Expr::Tag(name) => has_tag(tags, name),
            Expr::And(exprs) => exprs.iter().all(|e| e.matches(tags)),
            Expr::Or(exprs) => exprs.iter().any(|e| e.matches(tags)),
            Expr::Not(expr) => !expr.matches(tags),
        }
    }

    /// Every tag the rule mentions.
    pub fn tags(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_tags(&mut out);
        out
    }

    fn collect_tags<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::Tag(name) => out.push(name),
            Expr::And(exprs) | Expr::Or(exprs) => exprs.iter().for_each(|e| e.collect_tags(out)),
            Expr::Not(expr) => expr.collect_tags(out),
        }
    }
}

/// Prints the rule fully parenthesized, so the parsed precedence is visible.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let join = |f: &mut fmt::Formatter, exprs: &[Expr], op: &str| {
            write!(f, "(")?;
            for (i, e) in exprs.iter().enumerate() {
                if i > 0 {
                    write!(f, " {op} ")?;
                }
                write!(f, "{e}")?;
            }
            write!(f, ")")
        };
        match self {
            Expr::Tag(name) => write!(f, "{name}"),
            Expr::And(exprs) => join(f, exprs, "AND"),
            Expr::Or(exprs) => join(f, exprs, "OR"),
            Expr::Not(expr) => write!(f, "NOT {expr}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Tag(String),
    And,
    Or,
    Not,
    LParen,
    RParen,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Tag(name) => write!(f, "\"{name}\""),
            Token::And => write!(f, "AND"),
            Token::Or => write!(f, "OR"),
            Token::Not => write!(f, "NOT"),
            Token::LParen => write!(f, "\"(\""),
            Token::RParen => write!(f, "\")\""),
        }
    }
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '(' {
            tokens.push(Token::LParen);
            chars.next();
            continue;
        }
        if c == ')' {
            tokens.push(Token::RParen);
            chars.next();
            continue;
        }

        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '(' || c == ')' {
                break;
            }
            word.push(c);
            chars.next();
        }

        match word.to_uppercase().as_str() {
            "AND" => tokens.push(Token::And),
            "OR" => tokens.push(Token::Or),
            "NOT" => tokens.push(Token::Not),
            _ => tokens.push(Token::Tag(word)),
        }
    }

    tokens
}

fn parse_or(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let mut exprs = vec![parse_and(tokens, pos)?];
    while tokens.get(*pos) == Some(&Token::Or) {
        *pos += 1;
        exprs.push(parse_and(tokens, pos)?);
    }
    Ok(if exprs.len() == 1 { exprs.pop().unwrap() } else { Expr::Or(exprs) })
}

fn parse_and(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let mut exprs = vec![parse_not(tokens, pos)?];
    while tokens.get(*pos) == Some(&Token::And) {
        *pos += 1;
        exprs.push(parse_not(tokens, pos)?);
    }
    Ok(if exprs.len() == 1 { exprs.pop().unwrap() } else { Expr::And(exprs) })
}

fn parse_not(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    if tokens.get(*pos) == Some(&Token::Not) {
        *pos += 1;
        Ok(Expr::Not(Box::new(parse_not(tokens, pos)?)))
    } else {
        parse_atom(tokens, pos)
    }
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    match tokens.get(*pos) {
        None => Err("rule ends where a tag was expected".to_string()),
        Some(Token::Tag(name)) => {
            *pos += 1;
            Ok(Expr::Tag(name.clone()))
        }
        Some(Token::LParen) => {
            *pos += 1;
            let expr = parse_or(tokens, pos)?;
            if tokens.get(*pos) != Some(&Token::RParen) {
                return Err("missing closing parenthesis".to_string());
            }
            *pos += 1;
            Ok(expr)
        }
        Some(token) => Err(format!("unexpected {token} where a tag was expected")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn parse(s: &str) -> Expr {
        Expr::parse(s).unwrap()
    }

    #[test]
    fn precedence_is_not_then_and_then_or() {
        assert_eq!(parse("a OR b AND NOT c").to_string(), "(a OR (b AND NOT c))");
        assert_eq!(parse("(a OR b) AND c").to_string(), "((a OR b) AND c)");
        assert_eq!(parse("NOT NOT a").to_string(), "NOT NOT a");
        assert_eq!(parse("a and b or c").to_string(), "((a AND b) OR c)");
    }

    #[test]
    fn rejects_malformed_rules() {
        for bad in ["", "a b", "a AND", "AND a", "(a", "a)", "()", "NOT", "a OR OR b"] {
            assert!(Expr::parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn matches_hierarchically() {
        let entry = tags(&["topic/ai", "length/long", "state/unread"]);
        assert!(parse("topic").matches(&entry));
        assert!(parse("topic/ai AND length/long").matches(&entry));
        assert!(!parse("topic/ai AND NOT state/unread").matches(&entry));
        assert!(parse("topic/os OR (topic/ai AND NOT length/short)").matches(&entry));
        assert!(!parse("NOT topic").matches(&entry));
        assert!(parse("NOT topic").matches(&tags(&["state/unread"])));
    }

    #[test]
    fn lists_mentioned_tags() {
        assert_eq!(parse("a AND (b OR NOT c)").tags(), vec!["a", "b", "c"]);
    }
}
