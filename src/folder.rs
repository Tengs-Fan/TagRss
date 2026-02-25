use crate::models::Article;
use serde::{Deserialize, Serialize};

/// A folder is a named filter expression
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub filter: Expr,
}

/// Boolean expression over tags
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Expr {
    /// Match a tag (including hierarchical children)
    Tag { name: String },
    /// Logical AND
    And { exprs: Vec<Expr> },
    /// Logical OR
    Or { exprs: Vec<Expr> },
    /// Logical NOT
    Not { expr: Box<Expr> },
}

impl Expr {
    pub fn matches(&self, article: &Article) -> bool {
        match self {
            Expr::Tag { name } => article.match_tag(name),
            Expr::And { exprs } => exprs.iter().all(|e| e.matches(article)),
            Expr::Or { exprs } => exprs.iter().any(|e| e.matches(article)),
            Expr::Not { expr } => !expr.matches(article),
        }
    }

    /// Parse a simple expression DSL
    /// Examples:
    ///   "tech"                    -> Tag("tech")
    ///   "tech AND important"      -> And([Tag("tech"), Tag("important")])
    ///   "NOT life"                -> Not(Tag("life"))
    ///   "important AND NOT long"  -> And([Tag("important"), Not(Tag("long"))])
    pub fn parse(input: &str) -> Result<Expr, String> {
        let tokens = tokenize(input);
        if tokens.is_empty() {
            return Err("Empty expression".to_string());
        }
        parse_or(&tokens, &mut 0)
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

        // Read a word
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
    let mut left = parse_and(tokens, pos)?;

    while *pos < tokens.len() {
        if tokens[*pos] == Token::Or {
            *pos += 1;
            let right = parse_and(tokens, pos)?;
            left = match left {
                Expr::Or { mut exprs } => {
                    exprs.push(right);
                    Expr::Or { exprs }
                }
                _ => Expr::Or {
                    exprs: vec![left, right],
                },
            };
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_and(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let mut left = parse_not(tokens, pos)?;

    while *pos < tokens.len() {
        if tokens[*pos] == Token::And {
            *pos += 1;
            let right = parse_not(tokens, pos)?;
            left = match left {
                Expr::And { mut exprs } => {
                    exprs.push(right);
                    Expr::And { exprs }
                }
                _ => Expr::And {
                    exprs: vec![left, right],
                },
            };
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_not(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    if *pos < tokens.len() && tokens[*pos] == Token::Not {
        *pos += 1;
        let expr = parse_atom(tokens, pos)?;
        Ok(Expr::Not {
            expr: Box::new(expr),
        })
    } else {
        parse_atom(tokens, pos)
    }
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<Expr, String> {
    if *pos >= tokens.len() {
        return Err("Unexpected end of expression".to_string());
    }

    match &tokens[*pos] {
        Token::Tag(name) => {
            *pos += 1;
            Ok(Expr::Tag { name: name.clone() })
        }
        Token::LParen => {
            *pos += 1;
            let expr = parse_or(tokens, pos)?;
            if *pos >= tokens.len() || tokens[*pos] != Token::RParen {
                return Err("Missing closing parenthesis".to_string());
            }
            *pos += 1;
            Ok(expr)
        }
        _ => Err(format!("Unexpected token: {:?}", tokens[*pos])),
    }
}
