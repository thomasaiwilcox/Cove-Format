use crate::{
    ast::*, DiagnosticSeverity, ExplainMode, OqlDiagnostic, ResourceUseEstimate,
    COVE_OQL_LANGUAGE_VERSION,
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub fn parse_query(text: &str, options: ParseOptions) -> Result<ParsedQuery, Vec<OqlDiagnostic>> {
    if text.len() > options.resource_budget.maximum_query_bytes {
        return Err(vec![diagnostic_with_span(
            "E_RESOURCE_BUDGET_EXCEEDED",
            format!(
                "maximum_query_bytes budget exceeded: {} > {}",
                text.len(),
                options.resource_budget.maximum_query_bytes
            ),
            "parse",
            SourceSpan::new(0, text.len()),
        )]);
    }

    let lexed = lex(text)?;
    let language_version = lexed
        .language_version
        .clone()
        .unwrap_or_else(|| COVE_OQL_LANGUAGE_VERSION.to_string());
    if lexed.language_version.is_none() && !options.allow_implicit_language_version {
        return Err(vec![diagnostic_with_span(
            "E_PARSE",
            "missing required # cove-oql:0.1 directive",
            "parse",
            SourceSpan::new(0, 0),
        )]);
    }
    if let Some(required) = &options.required_language_version {
        if required != &language_version {
            return Err(vec![diagnostic_with_span(
                "E_UNSUPPORTED_CONSTRUCT",
                format!("unsupported Cove-OQL language version {language_version}"),
                "parse",
                SourceSpan::new(0, 0),
            )]);
        }
    }

    let query_text_fingerprint = sha256_hex(lexed.canonical_token_stream.as_bytes());
    let mut parser = Parser::new(lexed.tokens);
    let prefix_explain = parser.parse_prefix_explain()?;
    let root = parser.parse_root()?;
    let mut methods = Vec::new();
    while !parser.at(TokenDiscriminant::Eof) {
        parser.expect(
            TokenDiscriminant::Dot,
            "expected method chain starting with '.'",
        )?;
        methods.push(parser.parse_method()?);
    }
    if let Some(explain) = prefix_explain {
        methods.push(explain);
    }
    let span = match methods.last() {
        Some(method) => root.span.merge(method.span),
        None => root.span,
    };

    let resource_use = resource_use_for(text, &root, &methods);
    check_parse_budgets(&options.resource_budget, &resource_use)?;

    let canonical_ast = json!({
        "language_version": language_version,
        "root": root,
        "methods": methods,
    });
    let parsed_ast_fingerprint = sha256_hex(
        serde_json::to_string(&canonical_ast)
            .expect("canonical parsed AST serializes")
            .as_bytes(),
    );

    Ok(ParsedQuery {
        language_version,
        root,
        methods,
        span,
        resource_use,
        query_text_fingerprint,
        parsed_ast_fingerprint,
    })
}

#[derive(Debug, Clone)]
struct Lexed {
    tokens: Vec<Token>,
    language_version: Option<String>,
    canonical_token_stream: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Identifier(String, bool),
    String(String),
    Uuid(String),
    Binary(String),
    Integer(String),
    Decimal(String),
    True,
    False,
    Null,
    Dot,
    Comma,
    Colon,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Star,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenDiscriminant {
    Identifier,
    String,
    Uuid,
    Binary,
    Integer,
    Decimal,
    True,
    False,
    Null,
    Dot,
    Comma,
    Colon,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Star,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
}

impl TokenKind {
    fn discriminant(&self) -> TokenDiscriminant {
        match self {
            TokenKind::Identifier(_, _) => TokenDiscriminant::Identifier,
            TokenKind::String(_) => TokenDiscriminant::String,
            TokenKind::Uuid(_) => TokenDiscriminant::Uuid,
            TokenKind::Binary(_) => TokenDiscriminant::Binary,
            TokenKind::Integer(_) => TokenDiscriminant::Integer,
            TokenKind::Decimal(_) => TokenDiscriminant::Decimal,
            TokenKind::True => TokenDiscriminant::True,
            TokenKind::False => TokenDiscriminant::False,
            TokenKind::Null => TokenDiscriminant::Null,
            TokenKind::Dot => TokenDiscriminant::Dot,
            TokenKind::Comma => TokenDiscriminant::Comma,
            TokenKind::Colon => TokenDiscriminant::Colon,
            TokenKind::LParen => TokenDiscriminant::LParen,
            TokenKind::RParen => TokenDiscriminant::RParen,
            TokenKind::LBracket => TokenDiscriminant::LBracket,
            TokenKind::RBracket => TokenDiscriminant::RBracket,
            TokenKind::Star => TokenDiscriminant::Star,
            TokenKind::EqEq => TokenDiscriminant::EqEq,
            TokenKind::BangEq => TokenDiscriminant::BangEq,
            TokenKind::Lt => TokenDiscriminant::Lt,
            TokenKind::Le => TokenDiscriminant::Le,
            TokenKind::Gt => TokenDiscriminant::Gt,
            TokenKind::Ge => TokenDiscriminant::Ge,
            TokenKind::AndAnd => TokenDiscriminant::AndAnd,
            TokenKind::OrOr => TokenDiscriminant::OrOr,
            TokenKind::Bang => TokenDiscriminant::Bang,
            TokenKind::Eof => TokenDiscriminant::Eof,
        }
    }

    fn canonical(&self) -> String {
        match self {
            TokenKind::Identifier(value, _) => format!("id:{value}"),
            TokenKind::String(value) => format!("str:{value:?}"),
            TokenKind::Uuid(value) => format!("uuid:{value}"),
            TokenKind::Binary(value) => format!("bin:{value}"),
            TokenKind::Integer(value) => format!("int:{value}"),
            TokenKind::Decimal(value) => format!("dec:{value}"),
            TokenKind::True => "kw:true".into(),
            TokenKind::False => "kw:false".into(),
            TokenKind::Null => "kw:null".into(),
            TokenKind::Dot => ".".into(),
            TokenKind::Comma => ",".into(),
            TokenKind::Colon => ":".into(),
            TokenKind::LParen => "(".into(),
            TokenKind::RParen => ")".into(),
            TokenKind::LBracket => "[".into(),
            TokenKind::RBracket => "]".into(),
            TokenKind::Star => "*".into(),
            TokenKind::EqEq => "==".into(),
            TokenKind::BangEq => "!=".into(),
            TokenKind::Lt => "<".into(),
            TokenKind::Le => "<=".into(),
            TokenKind::Gt => ">".into(),
            TokenKind::Ge => ">=".into(),
            TokenKind::AndAnd => "&&".into(),
            TokenKind::OrOr => "||".into(),
            TokenKind::Bang => "!".into(),
            TokenKind::Eof => String::new(),
        }
    }
}

fn lex(text: &str) -> Result<Lexed, Vec<OqlDiagnostic>> {
    let mut tokens = Vec::new();
    let mut canonical = Vec::new();
    let mut diagnostics = Vec::new();
    let mut pos = 0usize;
    let mut language_version = None;
    let mut only_ws_before = true;

    while pos < text.len() {
        let ch = next_char(text, pos);
        if ch.is_whitespace() {
            pos += ch.len_utf8();
            continue;
        }

        if ch == '#' {
            let line_end = text[pos..]
                .find('\n')
                .map(|offset| pos + offset)
                .unwrap_or(text.len());
            let line = &text[pos..line_end];
            if only_ws_before && line.trim_start().starts_with("# cove-oql:") {
                let version = line
                    .trim_start()
                    .trim_start_matches("# cove-oql:")
                    .trim()
                    .to_string();
                language_version = Some(version);
            }
            pos = line_end;
            continue;
        }
        only_ws_before = false;

        let start = pos;
        let token = if text[pos..].starts_with("uuid\"") {
            let (value, end) = lex_string(text, pos + "uuid".len(), &mut diagnostics);
            pos = end;
            TokenKind::Uuid(value)
        } else if text[pos..].starts_with("b\"") {
            let (value, end) = lex_string(text, pos + 1, &mut diagnostics);
            pos = end;
            TokenKind::Binary(hex_encode(value.as_bytes()))
        } else if text[pos..].starts_with("x\"") {
            let (value, end) = lex_string(text, pos + 1, &mut diagnostics);
            pos = end;
            match canonical_hex_literal(&value) {
                Ok(hex) => TokenKind::Binary(hex),
                Err(message) => {
                    diagnostics.push(diagnostic_with_span(
                        "E_LITERAL",
                        message,
                        "parse",
                        SourceSpan::new(start, pos),
                    ));
                    continue;
                }
            }
        } else if is_identifier_start(ch) {
            pos += ch.len_utf8();
            while pos < text.len() {
                let next = next_char(text, pos);
                if !is_identifier_continue(next) {
                    break;
                }
                pos += next.len_utf8();
            }
            let value = &text[start..pos];
            match value {
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                "null" => TokenKind::Null,
                _ => TokenKind::Identifier(value.into(), false),
            }
        } else if ch == '`' {
            let (value, end) = lex_quoted_identifier(text, pos, &mut diagnostics);
            pos = end;
            TokenKind::Identifier(value, true)
        } else if ch == '"' {
            let (value, end) = lex_string(text, pos, &mut diagnostics);
            pos = end;
            TokenKind::String(value)
        } else if ch.is_ascii_digit()
            || (ch == '-'
                && pos + 1 < text.len()
                && next_char(text, pos + ch.len_utf8()).is_ascii_digit())
        {
            let (kind, end) = lex_number(text, pos);
            pos = end;
            kind
        } else {
            match ch {
                '.' => {
                    pos += 1;
                    TokenKind::Dot
                }
                ',' => {
                    pos += 1;
                    TokenKind::Comma
                }
                ':' => {
                    pos += 1;
                    TokenKind::Colon
                }
                '(' => {
                    pos += 1;
                    TokenKind::LParen
                }
                ')' => {
                    pos += 1;
                    TokenKind::RParen
                }
                '[' => {
                    pos += 1;
                    TokenKind::LBracket
                }
                ']' => {
                    pos += 1;
                    TokenKind::RBracket
                }
                '*' => {
                    pos += 1;
                    TokenKind::Star
                }
                '=' if text[pos..].starts_with("==") => {
                    pos += 2;
                    TokenKind::EqEq
                }
                '!' if text[pos..].starts_with("!=") => {
                    pos += 2;
                    TokenKind::BangEq
                }
                '<' if text[pos..].starts_with("<=") => {
                    pos += 2;
                    TokenKind::Le
                }
                '>' if text[pos..].starts_with(">=") => {
                    pos += 2;
                    TokenKind::Ge
                }
                '<' => {
                    pos += 1;
                    TokenKind::Lt
                }
                '>' => {
                    pos += 1;
                    TokenKind::Gt
                }
                '&' if text[pos..].starts_with("&&") => {
                    pos += 2;
                    TokenKind::AndAnd
                }
                '|' if text[pos..].starts_with("||") => {
                    pos += 2;
                    TokenKind::OrOr
                }
                '!' => {
                    pos += 1;
                    TokenKind::Bang
                }
                _ => {
                    diagnostics.push(diagnostic_with_span(
                        "E_PARSE",
                        format!("invalid token {ch:?}"),
                        "parse",
                        SourceSpan::new(start, start + ch.len_utf8()),
                    ));
                    pos += ch.len_utf8();
                    continue;
                }
            }
        };
        let span = SourceSpan::new(start, pos);
        canonical.push(token.canonical());
        tokens.push(Token { kind: token, span });
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: SourceSpan::new(text.len(), text.len()),
    });
    Ok(Lexed {
        tokens,
        language_version,
        canonical_token_stream: canonical.join(" "),
    })
}

fn lex_quoted_identifier(
    text: &str,
    start: usize,
    diagnostics: &mut Vec<OqlDiagnostic>,
) -> (String, usize) {
    let mut out = String::new();
    let mut pos = start + 1;
    while pos < text.len() {
        let ch = next_char(text, pos);
        if ch == '`' {
            return (out, pos + 1);
        }
        if ch == '\\' {
            pos += 1;
            if pos >= text.len() {
                break;
            }
            let escaped = next_char(text, pos);
            match escaped {
                '`' | '\\' => out.push(escaped),
                _ => out.push(escaped),
            }
            pos += escaped.len_utf8();
        } else {
            out.push(ch);
            pos += ch.len_utf8();
        }
    }
    diagnostics.push(diagnostic_with_span(
        "E_PARSE",
        "unterminated quoted identifier",
        "parse",
        SourceSpan::new(start, text.len()),
    ));
    (out, text.len())
}

fn lex_string(text: &str, start: usize, diagnostics: &mut Vec<OqlDiagnostic>) -> (String, usize) {
    let mut out = String::new();
    let mut pos = start + 1;
    while pos < text.len() {
        let ch = next_char(text, pos);
        if ch == '"' {
            return (out, pos + 1);
        }
        if ch == '\\' {
            pos += 1;
            if pos >= text.len() {
                break;
            }
            let escaped = next_char(text, pos);
            match escaped {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
            pos += escaped.len_utf8();
        } else {
            out.push(ch);
            pos += ch.len_utf8();
        }
    }
    diagnostics.push(diagnostic_with_span(
        "E_PARSE",
        "unterminated string literal",
        "parse",
        SourceSpan::new(start, text.len()),
    ));
    (out, text.len())
}

fn lex_number(text: &str, start: usize) -> (TokenKind, usize) {
    let mut pos = start;
    if text[pos..].starts_with('-') {
        pos += 1;
    }
    while pos < text.len() && next_char(text, pos).is_ascii_digit() {
        pos += 1;
    }
    let mut decimal = false;
    if pos < text.len() && next_char(text, pos) == '.' {
        let dot = pos;
        let after_dot = pos + 1;
        if after_dot < text.len() && next_char(text, after_dot).is_ascii_digit() {
            decimal = true;
            pos = after_dot;
            while pos < text.len() && next_char(text, pos).is_ascii_digit() {
                pos += 1;
            }
        } else {
            pos = dot;
        }
    }
    let value = text[start..pos].to_string();
    if decimal {
        (TokenKind::Decimal(value), pos)
    } else {
        (TokenKind::Integer(value), pos)
    }
}

fn canonical_hex_literal(value: &str) -> Result<String, &'static str> {
    if value.len() % 2 != 0 {
        return Err("hex binary literal must contain an even number of digits");
    }
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("hex binary literal contains a non-hex digit");
    }
    Ok(value.to_ascii_lowercase())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_prefix_explain(&mut self) -> Result<Option<Spanned<AstMethod>>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        let TokenKind::Identifier(value, _) = &self.current().kind else {
            return Ok(None);
        };
        if !value.eq_ignore_ascii_case("EXPLAIN") {
            return Ok(None);
        }
        self.advance();
        let mode = self
            .current_explain_mode_prefix()
            .map(|mode| {
                self.advance();
                mode
            })
            .unwrap_or(ExplainMode::Public);
        Ok(Some(Spanned::new(
            AstMethod::Explain(mode),
            SourceSpan::new(start, self.previous_span_end()),
        )))
    }

    fn parse_root(&mut self) -> Result<Spanned<AstRoot>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        if self.current_identifier_is("association") || self.current_is_association_direction() {
            let association = self.parse_association_expr_target()?;
            let span = SourceSpan::new(start, self.previous_span_end());
            Ok(Spanned::new(AstRoot::Association(association), span))
        } else if self.current_identifier_is("evidence") {
            let evidence = self.parse_evidence_expr()?;
            let span = SourceSpan::new(start, self.previous_span_end());
            Ok(Spanned::new(AstRoot::Evidence(evidence), span))
        } else if self.current_identifier_is("projection") {
            self.advance();
            self.expect(TokenDiscriminant::LParen, "expected '(' after projection")?;
            let projection = self.parse_identifier("expected projection identifier")?;
            self.expect(
                TokenDiscriminant::RParen,
                "expected ')' after projection root",
            )?;
            let span = SourceSpan::new(start, self.previous_span_end());
            Ok(Spanned::new(AstRoot::Projection(projection), span))
        } else {
            let identifier = self.parse_identifier("expected Cove-OQL root")?;
            let span = SourceSpan::new(start, self.previous_span_end());
            Ok(Spanned::new(AstRoot::Object(identifier), span))
        }
    }

    fn parse_method(&mut self) -> Result<Spanned<AstMethod>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        let name = self.parse_identifier("expected method name")?;
        match name.name.as_str() {
            "where" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after where")?;
                let predicate = self.parse_predicate()?;
                self.expect(
                    TokenDiscriminant::RParen,
                    "expected ')' after where predicate",
                )?;
                Ok(self.spanned(start, AstMethod::Where(predicate)))
            }
            "select" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after select")?;
                let mut items = Vec::new();
                if !self.at(TokenDiscriminant::RParen) {
                    loop {
                        items.push(self.parse_select_item()?);
                        if !self.match_token(TokenDiscriminant::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenDiscriminant::RParen, "expected ')' after select list")?;
                Ok(self.spanned(start, AstMethod::Select(items)))
            }
            "asOf" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after asOf")?;
                let bound = self.parse_time_bound()?;
                self.expect(TokenDiscriminant::RParen, "expected ')' after asOf bound")?;
                Ok(self.spanned(start, AstMethod::AsOf(bound)))
            }
            "branch" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after branch")?;
                let selector = self.parse_branch_selector()?;
                self.expect(
                    TokenDiscriminant::RParen,
                    "expected ')' after branch selector",
                )?;
                Ok(self.spanned(start, AstMethod::Branch(selector)))
            }
            "includeTombstones" => {
                self.expect(
                    TokenDiscriminant::LParen,
                    "expected '(' after includeTombstones",
                )?;
                let value = self.parse_boolean()?;
                self.expect(
                    TokenDiscriminant::RParen,
                    "expected ')' after includeTombstones",
                )?;
                Ok(self.spanned(start, AstMethod::IncludeTombstones(value)))
            }
            "history" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after history")?;
                let mode = if self.at(TokenDiscriminant::RParen) {
                    AstHistoryMode::States
                } else {
                    self.expect_identifier("mode")?;
                    self.expect(TokenDiscriminant::Colon, "expected ':' after history mode")?;
                    self.parse_history_mode()?
                };
                self.expect(TokenDiscriminant::RParen, "expected ')' after history")?;
                Ok(self.spanned(start, AstMethod::History(mode)))
            }
            "changes" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after changes")?;
                let from = self.parse_change_bound()?;
                self.expect(
                    TokenDiscriminant::Comma,
                    "expected ',' after changes from bound",
                )?;
                let to = self.parse_change_bound()?;
                let mode = if self.match_token(TokenDiscriminant::Comma) {
                    self.expect_identifier("mode")?;
                    self.expect(TokenDiscriminant::Colon, "expected ':' after changes mode")?;
                    self.parse_change_mode()?
                } else {
                    AstChangeMode::Records
                };
                self.expect(TokenDiscriminant::RParen, "expected ')' after changes")?;
                Ok(self.spanned(start, AstMethod::Changes { from, to, mode }))
            }
            "orderBy" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after orderBy")?;
                let expr = self.parse_expr()?;
                let direction = if self.match_token(TokenDiscriminant::Comma) {
                    if self.current_identifier_is("desc") {
                        self.advance();
                        AstOrderDirection::Desc
                    } else if self.current_identifier_is("asc") {
                        self.advance();
                        AstOrderDirection::Asc
                    } else {
                        return Err(self.err_current("expected asc or desc in orderBy"));
                    }
                } else {
                    AstOrderDirection::Asc
                };
                let nulls = if self.match_token(TokenDiscriminant::Comma) {
                    if self.current_identifier_is("nulls_first") {
                        self.advance();
                        AstNullOrdering::NullsFirst
                    } else if self.current_identifier_is("nulls_last") {
                        self.advance();
                        AstNullOrdering::NullsLast
                    } else {
                        return Err(self.err_current("expected nulls_first or nulls_last"));
                    }
                } else {
                    AstNullOrdering::Default
                };
                self.expect(TokenDiscriminant::RParen, "expected ')' after orderBy")?;
                Ok(self.spanned(
                    start,
                    AstMethod::OrderBy(AstOrderClause {
                        expr,
                        direction,
                        nulls,
                    }),
                ))
            }
            "take" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after take")?;
                let value = self.parse_u64("expected unsigned integer in take")?;
                self.expect(TokenDiscriminant::RParen, "expected ')' after take")?;
                Ok(self.spanned(start, AstMethod::Take(value)))
            }
            "skip" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after skip")?;
                let value = self.parse_u64("expected unsigned integer in skip")?;
                self.expect(TokenDiscriminant::RParen, "expected ')' after skip")?;
                Ok(self.spanned(start, AstMethod::Skip(value)))
            }
            "groupBy" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after groupBy")?;
                let mut exprs = Vec::new();
                if !self.at(TokenDiscriminant::RParen) {
                    loop {
                        exprs.push(self.parse_expr()?);
                        if !self.match_token(TokenDiscriminant::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenDiscriminant::RParen, "expected ')' after groupBy")?;
                Ok(self.spanned(start, AstMethod::GroupBy(exprs)))
            }
            "explain" => {
                self.expect(TokenDiscriminant::LParen, "expected '(' after explain")?;
                let mode = if self.at(TokenDiscriminant::RParen) {
                    ExplainMode::Public
                } else {
                    self.parse_explain_mode()?
                };
                self.expect(TokenDiscriminant::RParen, "expected ')' after explain")?;
                Ok(self.spanned(start, AstMethod::Explain(mode)))
            }
            other => Err(vec![diagnostic_with_span(
                "E_UNSUPPORTED_CONSTRUCT",
                format!("unsupported Cove-OQL method {other}"),
                "parse",
                SourceSpan::new(start, self.previous_span_end()),
            )]),
        }
    }

    fn parse_select_item(&mut self) -> Result<AstSelectItem, Vec<OqlDiagnostic>> {
        if self.peek_discriminant(0) == TokenDiscriminant::Identifier
            && self.peek_discriminant(1) == TokenDiscriminant::Colon
        {
            let alias = self.parse_identifier("expected select alias")?;
            self.expect(TokenDiscriminant::Colon, "expected ':' after select alias")?;
            let expr = self.parse_expr()?;
            Ok(AstSelectItem {
                alias: Some(alias),
                expr,
            })
        } else {
            Ok(AstSelectItem {
                alias: None,
                expr: self.parse_expr()?,
            })
        }
    }

    fn parse_predicate(&mut self) -> Result<Spanned<AstPredicate>, Vec<OqlDiagnostic>> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Spanned<AstPredicate>, Vec<OqlDiagnostic>> {
        let mut parts = vec![self.parse_and()?];
        while self.match_token(TokenDiscriminant::OrOr) {
            parts.push(self.parse_and()?);
        }
        if parts.len() == 1 {
            Ok(parts.remove(0))
        } else {
            let span = parts
                .iter()
                .fold(parts[0].span, |span, part| span.merge(part.span));
            Ok(Spanned::new(AstPredicate::Or(parts), span))
        }
    }

    fn parse_and(&mut self) -> Result<Spanned<AstPredicate>, Vec<OqlDiagnostic>> {
        let mut parts = vec![self.parse_not()?];
        while self.match_token(TokenDiscriminant::AndAnd) {
            parts.push(self.parse_not()?);
        }
        if parts.len() == 1 {
            Ok(parts.remove(0))
        } else {
            let span = parts
                .iter()
                .fold(parts[0].span, |span, part| span.merge(part.span));
            Ok(Spanned::new(AstPredicate::And(parts), span))
        }
    }

    fn parse_not(&mut self) -> Result<Spanned<AstPredicate>, Vec<OqlDiagnostic>> {
        if self.match_token(TokenDiscriminant::Bang) {
            let start = self.previous().span.start;
            let predicate = self.parse_not()?;
            let span = SourceSpan::new(start, predicate.span.end);
            Ok(Spanned::new(AstPredicate::Not(Box::new(predicate)), span))
        } else {
            self.parse_compare()
        }
    }

    fn parse_compare(&mut self) -> Result<Spanned<AstPredicate>, Vec<OqlDiagnostic>> {
        if self.current_identifier_is("exists")
            && self.peek_discriminant(1) == TokenDiscriminant::LParen
        {
            let start = self.current().span.start;
            self.advance();
            self.expect(TokenDiscriminant::LParen, "expected '(' after exists")?;
            let expr = self.parse_expr()?;
            self.expect(TokenDiscriminant::RParen, "expected ')' after exists")?;
            return Ok(Spanned::new(
                AstPredicate::Exists(expr),
                SourceSpan::new(start, self.previous_span_end()),
            ));
        }

        if self.match_token(TokenDiscriminant::LParen) {
            let start = self.previous().span.start;
            let inner = self.parse_predicate()?;
            self.expect(
                TokenDiscriminant::RParen,
                "expected ')' after grouped predicate",
            )?;
            let span = SourceSpan::new(start, self.previous_span_end());
            return Ok(Spanned::new(inner.node, span));
        }

        let left = self.parse_expr()?;
        if self.match_token(TokenDiscriminant::Dot) {
            if self.current_identifier_is("isNull") || self.current_identifier_is("isNotNull") {
                let negated = self.current_identifier_is("isNotNull");
                self.advance();
                self.expect(TokenDiscriminant::LParen, "expected '(' after null check")?;
                self.expect(TokenDiscriminant::RParen, "expected ')' after null check")?;
                let span = SourceSpan::new(left.span.start, self.previous_span_end());
                return Ok(Spanned::new(
                    AstPredicate::NullCheck {
                        expr: left,
                        negated,
                    },
                    span,
                ));
            }
            return Err(self.err_previous("expected null-check method after '.' in predicate"));
        }
        if self.current_identifier_is("in") {
            self.advance();
            self.expect(TokenDiscriminant::LBracket, "expected '[' after in")?;
            let mut values = Vec::new();
            if !self.at(TokenDiscriminant::RBracket) {
                loop {
                    values.push(self.parse_literal()?);
                    if !self.match_token(TokenDiscriminant::Comma) {
                        break;
                    }
                }
            }
            self.expect(TokenDiscriminant::RBracket, "expected ']' after in list")?;
            let span = SourceSpan::new(left.span.start, self.previous_span_end());
            return Ok(Spanned::new(
                AstPredicate::InList { expr: left, values },
                span,
            ));
        }
        if let Some(op) = self.match_compare_op() {
            let right = self.parse_expr()?;
            let span = SourceSpan::new(left.span.start, right.span.end);
            return Ok(Spanned::new(
                AstPredicate::Compare { left, op, right },
                span,
            ));
        }
        let span = left.span;
        Ok(Spanned::new(AstPredicate::BoolExpr(left), span))
    }

    fn parse_expr(&mut self) -> Result<Spanned<AstExpr>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        if self.match_token(TokenDiscriminant::LParen) {
            let expr = self.parse_expr()?;
            self.expect(TokenDiscriminant::RParen, "expected ')' after expression")?;
            return Ok(Spanned::new(
                expr.node,
                SourceSpan::new(start, self.previous_span_end()),
            ));
        }
        match &self.current().kind {
            TokenKind::String(_)
            | TokenKind::Uuid(_)
            | TokenKind::Binary(_)
            | TokenKind::Integer(_)
            | TokenKind::Decimal(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null => {
                let literal = self.parse_literal()?;
                Ok(literal.map(AstExpr::Literal))
            }
            TokenKind::Identifier(name, _) if name == "if" => self.parse_conditional_expr(),
            TokenKind::Identifier(name, _) if name == "association" => {
                let association = self.parse_association_expr_target()?;
                Ok(Spanned::new(
                    AstExpr::Association(association),
                    SourceSpan::new(start, self.previous_span_end()),
                ))
            }
            TokenKind::Identifier(name, _) if name == "evidence" => {
                let evidence = self.parse_evidence_expr()?;
                Ok(Spanned::new(
                    AstExpr::Evidence(evidence),
                    SourceSpan::new(start, self.previous_span_end()),
                ))
            }
            TokenKind::Identifier(name, _)
                if matches!(name.as_str(), "in" | "out" | "either")
                    && self.peek_discriminant(1) == TokenDiscriminant::LParen =>
            {
                let association = self.parse_association_expr_target()?;
                Ok(Spanned::new(
                    AstExpr::Association(association),
                    SourceSpan::new(start, self.previous_span_end()),
                ))
            }
            TokenKind::Identifier(name, _)
                if aggregate_name(name).is_some()
                    && self.peek_discriminant(1) == TokenDiscriminant::LParen =>
            {
                self.parse_aggregate_expr()
            }
            TokenKind::Identifier(_, _) => self.parse_path_or_function_expr(),
            _ => Err(self.err_current("expected expression")),
        }
    }

    fn parse_path_or_function_expr(&mut self) -> Result<Spanned<AstExpr>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        let first = self.parse_identifier("expected identifier")?;
        if self.match_token(TokenDiscriminant::LParen) {
            let args = self.parse_expr_args(TokenDiscriminant::RParen)?;
            return Ok(Spanned::new(
                AstExpr::FunctionCall { name: first, args },
                SourceSpan::new(start, self.previous_span_end()),
            ));
        }

        let mut parts = vec![first];
        loop {
            if self.peek_discriminant(0) == TokenDiscriminant::Dot
                && self.peek_discriminant(1) == TokenDiscriminant::Identifier
                && self.peek_discriminant(2) == TokenDiscriminant::LParen
            {
                break;
            }
            if self.peek_discriminant(0) == TokenDiscriminant::Dot
                && self.peek_discriminant(1) == TokenDiscriminant::Identifier
                && matches!(
                    self.peek_identifier(1).as_deref(),
                    Some("isNull" | "isNotNull")
                )
            {
                break;
            }
            if !self.match_token(TokenDiscriminant::Dot) {
                break;
            }
            parts.push(self.parse_identifier("expected identifier after '.'")?);
        }
        let mut expr = Spanned::new(
            AstExpr::Path(AstPath { parts }),
            SourceSpan::new(start, self.previous_span_end()),
        );

        if self.peek_discriminant(0) == TokenDiscriminant::Dot
            && self.peek_discriminant(1) == TokenDiscriminant::Identifier
            && self.peek_discriminant(2) == TokenDiscriminant::LParen
        {
            self.advance();
            let method = self.parse_identifier("expected function name after '.'")?;
            self.expect(
                TokenDiscriminant::LParen,
                "expected '(' after function name",
            )?;
            let mut args = vec![expr];
            if !self.at(TokenDiscriminant::RParen) {
                loop {
                    args.push(self.parse_expr()?);
                    if !self.match_token(TokenDiscriminant::Comma) {
                        break;
                    }
                }
            }
            self.expect(
                TokenDiscriminant::RParen,
                "expected ')' after function call",
            )?;
            expr = Spanned::new(
                AstExpr::FunctionCall { name: method, args },
                SourceSpan::new(start, self.previous_span_end()),
            );
        }
        Ok(expr)
    }

    fn parse_aggregate_expr(&mut self) -> Result<Spanned<AstExpr>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        let name_ident = self.parse_identifier("expected aggregate name")?;
        let name = aggregate_name(&name_ident.name).expect("guarded by caller");
        self.expect(
            TokenDiscriminant::LParen,
            "expected '(' after aggregate name",
        )?;
        let mut star = false;
        let arg = if self.match_token(TokenDiscriminant::Star) {
            star = true;
            None
        } else if self.at(TokenDiscriminant::RParen) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        self.expect(TokenDiscriminant::RParen, "expected ')' after aggregate")?;
        Ok(Spanned::new(
            AstExpr::AggregateCall { name, arg, star },
            SourceSpan::new(start, self.previous_span_end()),
        ))
    }

    fn parse_conditional_expr(&mut self) -> Result<Spanned<AstExpr>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        self.expect_identifier("if")?;
        self.expect(TokenDiscriminant::LParen, "expected '(' after if")?;
        let predicate = self.parse_predicate()?;
        self.expect(TokenDiscriminant::Comma, "expected ',' after if predicate")?;
        let then_expr = self.parse_expr()?;
        self.expect(
            TokenDiscriminant::Comma,
            "expected ',' after if then expression",
        )?;
        let else_expr = self.parse_expr()?;
        self.expect(TokenDiscriminant::RParen, "expected ')' after if")?;
        Ok(Spanned::new(
            AstExpr::Conditional {
                predicate: Box::new(predicate),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            SourceSpan::new(start, self.previous_span_end()),
        ))
    }

    fn parse_expr_args(
        &mut self,
        end: TokenDiscriminant,
    ) -> Result<Vec<Spanned<AstExpr>>, Vec<OqlDiagnostic>> {
        let mut args = Vec::new();
        if !self.at(end) {
            loop {
                args.push(self.parse_expr()?);
                if !self.match_token(TokenDiscriminant::Comma) {
                    break;
                }
            }
        }
        self.expect(end, "expected ')' after argument list")?;
        Ok(args)
    }

    fn parse_association_expr(
        &mut self,
        direction: Option<AstAssociationDirection>,
    ) -> Result<AstAssociationExpr, Vec<OqlDiagnostic>> {
        self.expect_identifier("association")?;
        self.expect(TokenDiscriminant::LParen, "expected '(' after association")?;
        let type_name = self.parse_identifier("expected association type")?;
        let mut role = None;
        let mut role_name = None;
        if self.match_token(TokenDiscriminant::Comma) {
            let role_key = self.parse_identifier("expected association role key")?;
            role = Some(match role_key.name.as_str() {
                "role" => AstAssociationRole::Role,
                "from" => AstAssociationRole::From,
                "to" => AstAssociationRole::To,
                _ => return Err(self.err_previous("expected role, from, or to")),
            });
            self.expect(TokenDiscriminant::Colon, "expected ':' after role key")?;
            role_name = Some(self.parse_identifier("expected association role value")?);
        }
        self.expect(TokenDiscriminant::RParen, "expected ')' after association")?;
        Ok(AstAssociationExpr {
            type_name,
            direction,
            role,
            role_name,
        })
    }

    fn parse_association_expr_target(&mut self) -> Result<AstAssociationExpr, Vec<OqlDiagnostic>> {
        if self.current_identifier_is("association") {
            return self.parse_association_expr(None);
        }
        let Some(direction) = self.current_association_direction() else {
            return Err(self.err_current("expected association expression"));
        };
        self.advance();
        self.expect(
            TokenDiscriminant::LParen,
            "expected '(' after association direction",
        )?;
        let association = self.parse_association_expr(Some(direction))?;
        self.expect(
            TokenDiscriminant::RParen,
            "expected ')' after association direction",
        )?;
        Ok(association)
    }

    fn parse_evidence_expr(&mut self) -> Result<AstEvidenceExpr, Vec<OqlDiagnostic>> {
        self.expect_identifier("evidence")?;
        self.expect(TokenDiscriminant::LParen, "expected '(' after evidence")?;
        let mut target = None;
        let mut grain = None;
        if !self.at(TokenDiscriminant::RParen) {
            loop {
                if self.current_identifier_is("grain")
                    && self.peek_discriminant(1) == TokenDiscriminant::Colon
                {
                    self.advance();
                    self.expect(TokenDiscriminant::Colon, "expected ':' after grain")?;
                    grain = Some(self.parse_evidence_grain()?);
                } else {
                    target = Some(self.parse_evidence_target()?);
                }
                if !self.match_token(TokenDiscriminant::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenDiscriminant::RParen, "expected ')' after evidence")?;
        Ok(AstEvidenceExpr { target, grain })
    }

    fn parse_evidence_target(&mut self) -> Result<AstEvidenceTarget, Vec<OqlDiagnostic>> {
        if self.current_identifier_is("self") {
            self.advance();
            return Ok(AstEvidenceTarget::SelfTarget);
        }
        if self.current_identifier_is("association") || self.current_is_association_direction() {
            return Ok(AstEvidenceTarget::Association(Box::new(
                self.parse_association_expr_target()?,
            )));
        }
        if self.current_identifier_is("projection")
            && self.peek_discriminant(1) == TokenDiscriminant::LParen
        {
            self.advance();
            self.expect(TokenDiscriminant::LParen, "expected '(' after projection")?;
            let id = self.parse_identifier("expected projection identifier")?;
            self.expect(TokenDiscriminant::RParen, "expected ')' after projection")?;
            return Ok(AstEvidenceTarget::Projection(id));
        }
        let path = self.parse_path()?;
        Ok(AstEvidenceTarget::Path(path.node))
    }

    fn parse_path(&mut self) -> Result<Spanned<AstPath>, Vec<OqlDiagnostic>> {
        let start = self.current().span.start;
        let mut parts = vec![self.parse_identifier("expected path identifier")?];
        while self.match_token(TokenDiscriminant::Dot) {
            parts.push(self.parse_identifier("expected identifier after '.'")?);
        }
        Ok(Spanned::new(
            AstPath { parts },
            SourceSpan::new(start, self.previous_span_end()),
        ))
    }

    fn parse_time_bound(&mut self) -> Result<AstTimeBound, Vec<OqlDiagnostic>> {
        let key = self.parse_identifier("expected asOf bound key")?;
        self.expect(
            TokenDiscriminant::Colon,
            "expected ':' after asOf bound key",
        )?;
        if key.name == "csn" {
            Ok(AstTimeBound::Csn(
                self.parse_u64("expected unsigned CSN after csn:")?,
            ))
        } else {
            let role = time_role(&key.name).ok_or_else(|| {
                self.err_previous(
                    "expected time, commit_time, valid_time, observed_time, source_event_time, or association_valid_time",
                )
            })?;
            let timestamp = self.parse_timestamp_literal()?;
            Ok(AstTimeBound::Timestamp { role, timestamp })
        }
    }

    fn parse_change_bound(&mut self) -> Result<AstChangeBound, Vec<OqlDiagnostic>> {
        if matches!(self.peek_identifier(0).as_deref(), Some("from" | "to"))
            && self.peek_discriminant(1) == TokenDiscriminant::Colon
        {
            self.advance();
            self.expect(
                TokenDiscriminant::Colon,
                "expected ':' after changes bound label",
            )?;
            if matches!(self.current().kind, TokenKind::Integer(_)) {
                return Ok(AstChangeBound::Csn(
                    self.parse_u64("expected unsigned CSN in changes bound")?,
                ));
            }
            if matches!(self.current().kind, TokenKind::String(_)) {
                return Ok(AstChangeBound::Timestamp {
                    role: AstTimeRole::CommitTime,
                    timestamp: self.parse_timestamp_literal()?,
                });
            }
        }

        let key = self.parse_identifier("expected changes bound key")?;
        self.expect(
            TokenDiscriminant::Colon,
            "expected ':' after changes bound key",
        )?;
        if key.name == "csn" {
            Ok(AstChangeBound::Csn(
                self.parse_u64("expected unsigned CSN after csn:")?,
            ))
        } else {
            let role =
                time_role(&key.name).ok_or_else(|| self.err_previous("expected time role"))?;
            Ok(AstChangeBound::Timestamp {
                role,
                timestamp: self.parse_timestamp_literal()?,
            })
        }
    }

    fn parse_branch_selector(&mut self) -> Result<AstBranchSelector, Vec<OqlDiagnostic>> {
        match self.current().kind.clone() {
            TokenKind::Identifier(_, _) => Ok(AstBranchSelector::Identifier(
                self.parse_identifier("expected branch selector")?,
            )),
            TokenKind::String(value) => {
                self.advance();
                Ok(AstBranchSelector::String(value))
            }
            TokenKind::Integer(_) => Ok(AstBranchSelector::UInt(
                self.parse_u64("expected unsigned branch key")?,
            )),
            _ => Err(self.err_current("expected branch selector")),
        }
    }

    fn parse_literal(&mut self) -> Result<Spanned<AstLiteral>, Vec<OqlDiagnostic>> {
        let token = self.current().clone();
        self.advance();
        let literal = match token.kind {
            TokenKind::String(value) => AstLiteral::String(value),
            TokenKind::Uuid(value) => AstLiteral::Uuid(value),
            TokenKind::Binary(value) => AstLiteral::Binary(value),
            TokenKind::Integer(value) => AstLiteral::Integer(value),
            TokenKind::Decimal(value) => AstLiteral::Decimal(value),
            TokenKind::True => AstLiteral::Boolean(true),
            TokenKind::False => AstLiteral::Boolean(false),
            TokenKind::Null => AstLiteral::Null,
            _ => return Err(vec![diagnostic("E_PARSE", "expected literal", "parse")]),
        };
        Ok(Spanned::new(literal, token.span))
    }

    fn parse_timestamp_literal(&mut self) -> Result<String, Vec<OqlDiagnostic>> {
        match self.current().kind.clone() {
            TokenKind::String(value) => {
                self.advance();
                Ok(value)
            }
            _ => Err(self.err_current("expected RFC3339 timestamp string literal")),
        }
    }

    fn parse_boolean(&mut self) -> Result<bool, Vec<OqlDiagnostic>> {
        if self.match_token(TokenDiscriminant::True) {
            Ok(true)
        } else if self.match_token(TokenDiscriminant::False) {
            Ok(false)
        } else {
            Err(self.err_current("expected boolean literal"))
        }
    }

    fn parse_history_mode(&mut self) -> Result<AstHistoryMode, Vec<OqlDiagnostic>> {
        let ident = self.parse_identifier("expected history mode")?;
        match ident.name.as_str() {
            "records" => Ok(AstHistoryMode::Records),
            "states" => Ok(AstHistoryMode::States),
            "records_and_states" => Ok(AstHistoryMode::RecordsAndStates),
            _ => {
                Err(self
                    .err_previous_code("E_UNSUPPORTED_HISTORY_MODE", "unsupported history mode"))
            }
        }
    }

    fn parse_change_mode(&mut self) -> Result<AstChangeMode, Vec<OqlDiagnostic>> {
        let ident = self.parse_identifier("expected changes mode")?;
        match ident.name.as_str() {
            "records" => Ok(AstChangeMode::Records),
            "state_transitions" => Ok(AstChangeMode::StateTransitions),
            "property_diffs" => Ok(AstChangeMode::PropertyDiffs),
            "final_objects" => Ok(AstChangeMode::FinalObjects),
            _ => {
                Err(self.err_previous_code("E_UNSUPPORTED_CHANGE_MODE", "unsupported changes mode"))
            }
        }
    }

    fn parse_evidence_grain(&mut self) -> Result<AstEvidenceGrain, Vec<OqlDiagnostic>> {
        let ident = self.parse_identifier("expected evidence grain")?;
        match ident.name.as_str() {
            "object" => Ok(AstEvidenceGrain::Object),
            "property" => Ok(AstEvidenceGrain::Property),
            "association" => Ok(AstEvidenceGrain::Association),
            "row" => Ok(AstEvidenceGrain::Row),
            "source" => Ok(AstEvidenceGrain::Source),
            _ => {
                Err(self
                    .err_previous_code("E_UNKNOWN_EVIDENCE_GRAIN", "unsupported evidence grain"))
            }
        }
    }

    fn parse_explain_mode(&mut self) -> Result<ExplainMode, Vec<OqlDiagnostic>> {
        let value = match self.current().kind.clone() {
            TokenKind::Identifier(value, _) | TokenKind::String(value) => {
                self.advance();
                value
            }
            _ => return Err(self.err_current("expected explain mode")),
        };
        explain_mode_from_str(&value).ok_or_else(|| self.err_previous("unsupported explain mode"))
    }

    fn current_explain_mode_prefix(&self) -> Option<ExplainMode> {
        if matches!(
            self.peek_discriminant(1),
            TokenDiscriminant::Dot | TokenDiscriminant::LParen
        ) {
            return None;
        }
        let value = match &self.current().kind {
            TokenKind::Identifier(value, _) | TokenKind::String(value) => value,
            _ => return None,
        };
        explain_mode_from_str(value)
    }

    fn parse_u64(&mut self, message: &'static str) -> Result<u64, Vec<OqlDiagnostic>> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Integer(value) if !value.starts_with('-') => {
                self.advance();
                value
                    .parse::<u64>()
                    .map_err(|_| self.err_previous("unsigned integer is out of range"))
            }
            _ => Err(self.err_current(message)),
        }
    }

    fn parse_identifier(
        &mut self,
        message: &'static str,
    ) -> Result<AstIdentifier, Vec<OqlDiagnostic>> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Identifier(value, quoted) => {
                self.advance();
                Ok(AstIdentifier::new(value, quoted))
            }
            _ => Err(self.err_current(message)),
        }
    }

    fn expect_identifier(&mut self, expected: &str) -> Result<(), Vec<OqlDiagnostic>> {
        if self.current_identifier_is(expected) {
            self.advance();
            Ok(())
        } else {
            Err(self.err_current(format!("expected {expected}")))
        }
    }

    fn match_compare_op(&mut self) -> Option<AstCompareOp> {
        let op = match self.current().kind {
            TokenKind::EqEq => AstCompareOp::Eq,
            TokenKind::BangEq => AstCompareOp::Ne,
            TokenKind::Lt => AstCompareOp::Lt,
            TokenKind::Le => AstCompareOp::Le,
            TokenKind::Gt => AstCompareOp::Gt,
            TokenKind::Ge => AstCompareOp::Ge,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    fn expect(
        &mut self,
        kind: TokenDiscriminant,
        message: impl Into<String>,
    ) -> Result<(), Vec<OqlDiagnostic>> {
        if self.at(kind) {
            self.advance();
            Ok(())
        } else {
            Err(self.err_current(message))
        }
    }

    fn match_token(&mut self, kind: TokenDiscriminant) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenDiscriminant) -> bool {
        self.current().kind.discriminant() == kind
    }

    fn current_identifier_is(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Identifier(value, _) if value == expected)
    }

    fn current_is_association_direction(&self) -> bool {
        self.current_association_direction().is_some()
            && self.peek_discriminant(1) == TokenDiscriminant::LParen
    }

    fn current_association_direction(&self) -> Option<AstAssociationDirection> {
        match &self.current().kind {
            TokenKind::Identifier(value, _) if value == "in" => Some(AstAssociationDirection::In),
            TokenKind::Identifier(value, _) if value == "out" => Some(AstAssociationDirection::Out),
            TokenKind::Identifier(value, _) if value == "either" => {
                Some(AstAssociationDirection::Either)
            }
            _ => None,
        }
    }

    fn peek_identifier(&self, offset: usize) -> Option<String> {
        match &self.tokens[self.pos + offset].kind {
            TokenKind::Identifier(value, _) => Some(value.clone()),
            _ => None,
        }
    }

    fn peek_discriminant(&self, offset: usize) -> TokenDiscriminant {
        self.tokens
            .get(self.pos + offset)
            .map(|token| token.kind.discriminant())
            .unwrap_or(TokenDiscriminant::Eof)
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.pos.saturating_sub(1)]
    }

    fn previous_span_end(&self) -> usize {
        self.previous().span.end
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
    }

    fn spanned(&self, start: usize, method: AstMethod) -> Spanned<AstMethod> {
        Spanned::new(method, SourceSpan::new(start, self.previous_span_end()))
    }

    fn err_current(&self, message: impl Into<String>) -> Vec<OqlDiagnostic> {
        vec![diagnostic_with_span(
            "E_PARSE",
            message,
            "parse",
            self.current().span,
        )]
    }

    fn err_previous(&self, message: impl Into<String>) -> Vec<OqlDiagnostic> {
        vec![diagnostic_with_span(
            "E_PARSE",
            message,
            "parse",
            self.previous().span,
        )]
    }

    fn err_previous_code(
        &self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Vec<OqlDiagnostic> {
        vec![diagnostic_with_span(
            code,
            message,
            "parse",
            self.previous().span,
        )]
    }
}

fn explain_mode_from_str(value: &str) -> Option<ExplainMode> {
    match value.to_ascii_lowercase().as_str() {
        "public" => Some(ExplainMode::Public),
        "developer" => Some(ExplainMode::Developer),
        "proof" => Some(ExplainMode::Proof),
        "coded" => Some(ExplainMode::Coded),
        "forensic" => Some(ExplainMode::Forensic),
        _ => None,
    }
}

fn resource_use_for(
    text: &str,
    root: &Spanned<AstRoot>,
    methods: &[Spanned<AstMethod>],
) -> ResourceUseEstimate {
    let mut use_estimate = ResourceUseEstimate {
        query_bytes: Some(text.len()),
        method_count: Some(methods.len()),
        ast_depth: Some(root_depth(root) + methods.iter().map(method_depth).max().unwrap_or(0)),
        in_list_size: Some(methods.iter().map(method_in_list_size).sum()),
        disjunction_count: Some(methods.iter().map(method_disjunction_count).sum()),
        output_columns: Some(
            methods
                .iter()
                .find_map(|method| match &method.node {
                    AstMethod::Select(items) => Some(items.len()),
                    _ => None,
                })
                .unwrap_or(0),
        ),
        ..ResourceUseEstimate::default()
    };
    if methods
        .iter()
        .all(|method| !matches!(method.node, AstMethod::Take(_)))
    {
        use_estimate.rows_without_explicit_take = Some(1);
    }
    use_estimate
}

fn root_depth(root: &Spanned<AstRoot>) -> usize {
    match &root.node {
        AstRoot::Evidence(evidence) => 1 + evidence_depth(evidence),
        _ => 1,
    }
}

fn method_depth(method: &Spanned<AstMethod>) -> usize {
    match &method.node {
        AstMethod::Where(predicate) => predicate_depth(predicate),
        AstMethod::Select(items) => items
            .iter()
            .map(|item| expr_depth(&item.expr))
            .max()
            .unwrap_or(1),
        AstMethod::OrderBy(order) => expr_depth(&order.expr),
        AstMethod::GroupBy(exprs) => exprs.iter().map(expr_depth).max().unwrap_or(1),
        _ => 1,
    }
}

fn predicate_depth(predicate: &Spanned<AstPredicate>) -> usize {
    match &predicate.node {
        AstPredicate::Compare { left, right, .. } => 1 + expr_depth(left).max(expr_depth(right)),
        AstPredicate::InList { expr, .. } => 1 + expr_depth(expr),
        AstPredicate::NullCheck { expr, .. } | AstPredicate::BoolExpr(expr) => 1 + expr_depth(expr),
        AstPredicate::Exists(expr) => 1 + expr_depth(expr),
        AstPredicate::Not(inner) => 1 + predicate_depth(inner),
        AstPredicate::And(parts) | AstPredicate::Or(parts) => {
            1 + parts.iter().map(predicate_depth).max().unwrap_or(0)
        }
    }
}

fn expr_depth(expr: &Spanned<AstExpr>) -> usize {
    match &expr.node {
        AstExpr::FunctionCall { args, .. } => 1 + args.iter().map(expr_depth).max().unwrap_or(0),
        AstExpr::AggregateCall { arg, .. } => 1 + arg.as_deref().map(expr_depth).unwrap_or(0),
        AstExpr::Evidence(evidence) => 1 + evidence_depth(evidence),
        AstExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
        } => {
            1 + predicate_depth(predicate)
                .max(expr_depth(then_expr))
                .max(expr_depth(else_expr))
        }
        _ => 1,
    }
}

fn evidence_depth(evidence: &AstEvidenceExpr) -> usize {
    match &evidence.target {
        Some(AstEvidenceTarget::Association(_)) | Some(AstEvidenceTarget::Projection(_)) => 2,
        Some(AstEvidenceTarget::Path(_)) | Some(AstEvidenceTarget::SelfTarget) | None => 1,
    }
}

fn method_in_list_size(method: &Spanned<AstMethod>) -> usize {
    match &method.node {
        AstMethod::Where(predicate) => predicate_in_list_size(predicate),
        _ => 0,
    }
}

fn predicate_in_list_size(predicate: &Spanned<AstPredicate>) -> usize {
    match &predicate.node {
        AstPredicate::InList { values, .. } => values.len(),
        AstPredicate::Not(inner) => predicate_in_list_size(inner),
        AstPredicate::And(parts) | AstPredicate::Or(parts) => {
            parts.iter().map(predicate_in_list_size).sum()
        }
        _ => 0,
    }
}

fn method_disjunction_count(method: &Spanned<AstMethod>) -> usize {
    match &method.node {
        AstMethod::Where(predicate) => predicate_disjunction_count(predicate),
        _ => 0,
    }
}

fn predicate_disjunction_count(predicate: &Spanned<AstPredicate>) -> usize {
    match &predicate.node {
        AstPredicate::Or(parts) => {
            parts.len().saturating_sub(1)
                + parts.iter().map(predicate_disjunction_count).sum::<usize>()
        }
        AstPredicate::And(parts) => parts.iter().map(predicate_disjunction_count).sum(),
        AstPredicate::Not(inner) => predicate_disjunction_count(inner),
        _ => 0,
    }
}

fn check_parse_budgets(
    budget: &crate::ResourceBudgetPolicy,
    usage: &ResourceUseEstimate,
) -> Result<(), Vec<OqlDiagnostic>> {
    let checks = [
        (
            "maximum_ast_depth",
            usage.ast_depth,
            budget.maximum_ast_depth,
        ),
        (
            "maximum_method_count",
            usage.method_count,
            budget.maximum_method_count,
        ),
        (
            "maximum_in_list_size",
            usage.in_list_size,
            budget.maximum_in_list_size,
        ),
        (
            "maximum_disjunction_count",
            usage.disjunction_count,
            budget.maximum_disjunction_count,
        ),
        (
            "maximum_output_columns",
            usage.output_columns,
            budget.maximum_output_columns,
        ),
    ];
    for (field, value, limit) in checks {
        if let Some(value) = value {
            if value > limit {
                return Err(vec![diagnostic(
                    "E_RESOURCE_BUDGET_EXCEEDED",
                    format!("{field} budget exceeded: {value} > {limit}"),
                    "parse",
                )]);
            }
        }
    }
    Ok(())
}

fn aggregate_name(name: &str) -> Option<AstAggregateName> {
    match name {
        "count" => Some(AstAggregateName::Count),
        "min" => Some(AstAggregateName::Min),
        "max" => Some(AstAggregateName::Max),
        "sum" => Some(AstAggregateName::Sum),
        "avg" => Some(AstAggregateName::Avg),
        "exists" => Some(AstAggregateName::Exists),
        "distinct_count" => Some(AstAggregateName::DistinctCount),
        _ => None,
    }
}

fn time_role(name: &str) -> Option<AstTimeRole> {
    match name {
        "time" => Some(AstTimeRole::Time),
        "commit_time" => Some(AstTimeRole::CommitTime),
        "valid_time" => Some(AstTimeRole::ValidTime),
        "observed_time" => Some(AstTimeRole::ObservedTime),
        "source_event_time" => Some(AstTimeRole::SourceEventTime),
        "association_valid_time" => Some(AstTimeRole::AssociationValidTime),
        _ => None,
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn next_char(text: &str, pos: usize) -> char {
    text[pos..].chars().next().expect("pos is on char boundary")
}

fn diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
) -> OqlDiagnostic {
    OqlDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: phase.into(),
        safe_details: json!({}),
        redacted: true,
    }
}

fn diagnostic_with_span(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    span: SourceSpan,
) -> OqlDiagnostic {
    OqlDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: phase.into(),
        safe_details: json!({
            "span": {
                "start": span.start,
                "end": span.end,
            }
        }),
        redacted: true,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_ignores_directive_and_whitespace_for_query_fingerprint() {
        let a = parse_query(
            "# cove-oql:0.1\nPerson.where(name == \"Ada\")",
            ParseOptions::default(),
        )
        .unwrap();
        let b = parse_query(
            "Person  . where ( name == \"Ada\" )",
            ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(a.parsed_ast_fingerprint, b.parsed_ast_fingerprint);
    }

    #[test]
    fn quoted_identifier_does_not_change_ast_fingerprint() {
        let a = parse_query("Person.select(name)", ParseOptions::default()).unwrap();
        let b = parse_query("`Person`.select(`name`)", ParseOptions::default()).unwrap();
        assert_eq!(a.parsed_ast_fingerprint, b.parsed_ast_fingerprint);
    }

    #[test]
    fn parses_core_methods_and_precedence() {
        let parsed = parse_query(
            "Person.where(!active || name == \"Ada\" && age >= 40).select(goid, name).orderBy(name, desc, nulls_last).take(10)",
            ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(parsed.methods.len(), 4);
        let AstMethod::Where(predicate) = &parsed.methods[0].node else {
            panic!("expected where");
        };
        assert!(matches!(predicate.node, AstPredicate::Or(_)));
    }

    #[test]
    fn parses_association_projection_and_evidence_roots() {
        parse_query(
            "association(CustomerPlacedOrder).select(source_goid, target_goid)",
            ParseOptions::default(),
        )
        .unwrap();
        parse_query(
            "projection(people_projection).where(name == \"Ada\")",
            ParseOptions::default(),
        )
        .unwrap();
        parse_query(
            "evidence(Person, grain: object).where(source_id in [\"crm\", \"dir\"])",
            ParseOptions::default(),
        )
        .unwrap();
        let err = parse_query(
            "evidence(Person, grain: mystery).select(source_id)",
            ParseOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err[0].code, "E_UNKNOWN_EVIDENCE_GRAIN");
        let err =
            parse_query("Person.history(mode: snapshot)", ParseOptions::default()).unwrap_err();
        assert_eq!(err[0].code, "E_UNSUPPORTED_HISTORY_MODE");
        let err = parse_query(
            "Person.changes(from: 1, to: 2, mode: snapshots)",
            ParseOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err[0].code, "E_UNSUPPORTED_CHANGE_MODE");
    }

    #[test]
    fn parses_binary_literals_and_rejects_malformed_hex() {
        parse_query("Person.where(blob == x\"00\")", ParseOptions::default()).unwrap();
        parse_query("Person.where(blob == b\"ok\")", ParseOptions::default()).unwrap();
        let err = parse_query("Person.where(blob == x\"0\")", ParseOptions::default()).unwrap_err();
        assert_eq!(err[0].code, "E_LITERAL");
    }

    #[test]
    fn enforces_parse_resource_budgets() {
        let mut options = ParseOptions::default();
        options.resource_budget.maximum_output_columns = 1;
        let err = parse_query("Person.select(a, b)", options).unwrap_err();
        assert_eq!(err[0].code, "E_RESOURCE_BUDGET_EXCEEDED");
    }
}
