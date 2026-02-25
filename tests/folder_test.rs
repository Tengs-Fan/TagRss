use tagrss::folder::Expr;

#[test]
fn test_parse_simple() {
    let expr = Expr::parse("tech").unwrap();
    assert!(matches!(expr, Expr::Tag { name } if name == "tech"));
}

#[test]
fn test_parse_and() {
    let expr = Expr::parse("tech AND important").unwrap();
    assert!(matches!(expr, Expr::And { exprs } if exprs.len() == 2));
}

#[test]
fn test_parse_not() {
    let expr = Expr::parse("NOT life").unwrap();
    assert!(matches!(expr, Expr::Not { .. }));
}

#[test]
fn test_parse_complex() {
    let expr = Expr::parse("important AND NOT life AND NOT long").unwrap();
    if let Expr::And { exprs } = expr {
        assert_eq!(exprs.len(), 3);
    } else {
        panic!("Expected And expression");
    }
}

#[test]
fn test_parse_parens() {
    let expr = Expr::parse("tech AND (news OR blog)").unwrap();
    assert!(matches!(expr, Expr::And { .. }));
}

#[test]
fn test_parse_or() {
    let expr = Expr::parse("tech OR news").unwrap();
    assert!(matches!(expr, Expr::Or { exprs } if exprs.len() == 2));
}

#[test]
fn test_parse_nested() {
    let expr = Expr::parse("(tech AND important) OR (news AND fresh)").unwrap();
    if let Expr::Or { exprs } = expr {
        assert_eq!(exprs.len(), 2);
        assert!(matches!(&exprs[0], Expr::And { .. }));
        assert!(matches!(&exprs[1], Expr::And { .. }));
    } else {
        panic!("Expected Or expression");
    }
}

#[test]
fn test_parse_empty_error() {
    let result = Expr::parse("");
    assert!(result.is_err());
}

#[test]
fn test_parse_hierarchical_tag() {
    let expr = Expr::parse("tech/ai/llm").unwrap();
    assert!(matches!(expr, Expr::Tag { name } if name == "tech/ai/llm"));
}
