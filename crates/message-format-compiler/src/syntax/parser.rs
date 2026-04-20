// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grammar-driven parser following TR35 ABNF productions.

use crate::compile::CompileError;
use crate::syntax::ast::{
    AttributeNode, CallExpressionNode, CallOperandNode, DeclarationKind, DeclarationNode,
    DeclarationPayloadNode, ExpressionDiagnosticHint, ExpressionKindNode, ExpressionNode,
    ExpressionPayloadNode, FunctionSpecNode, LiteralExpressionNode, MarkupKind, MarkupNode,
    MatchParseError, MatchVariantNode, OptionNode, OptionValue, PatternNode, PatternSegmentNode,
    SyntaxDocument, VarExpressionNode, VariantKeyNode,
};
use crate::syntax::charset::{is_mf2_whitespace, is_name_char, is_name_start, is_quoted_char};
use crate::syntax::ident::is_bidi_control;

// ─── Parser struct ───────────────────────────────────────────────────────────

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    // ─── Cursor primitives ───────────────────────────────────────────

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        if self.src[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn remaining(&self) -> &'a str {
        &self.src[self.pos..]
    }

    /// Skip ABNF `s = *bidi ws o` — returns true only if at least one actual
    /// `ws` character (SP, HTAB, CR, LF, U+3000) was consumed. Bidi controls
    /// alone do NOT satisfy mandatory whitespace and are not consumed.
    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        // Skip leading bidi controls tentatively
        while let Some(ch) = self.peek() {
            if is_bidi_control(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        // Must see at least one actual ws char
        if !self.peek().is_some_and(is_mf2_whitespace) {
            // No ws found — backtrack past any bidi we consumed
            self.pos = start;
            return false;
        }
        // Consume all remaining ws and bidi
        while let Some(ch) = self.peek() {
            if is_mf2_whitespace(ch) || is_bidi_control(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        true
    }

    /// Skip optional whitespace (ABNF `o = *(ws / bidi)`) — consumes any
    /// combination of ws and bidi controls, including bidi-only sequences.
    fn skip_optional_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if is_mf2_whitespace(ch) || is_bidi_control(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    /// Skip only actual whitespace characters (ABNF `s`: SP, HTAB, CR, LF, U+3000),
    /// NOT bidi controls. Use this where bidi controls should not act as separators.
    fn skip_actual_whitespace(&mut self) -> bool {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if is_mf2_whitespace(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        self.pos > start
    }

    // ─── Name/identifier parsing ─────────────────────────────────────

    /// Parse ABNF `name`: `name-start *name-char`.
    fn parse_name(&mut self) -> Option<&'a str> {
        let start = self.pos;
        let ch = self.peek()?;
        if !is_name_start(ch) {
            return None;
        }
        self.pos += ch.len_utf8();
        while let Some(ch) = self.peek() {
            if is_name_char(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        Some(&self.src[start..self.pos])
    }

    /// Parse ABNF `identifier`: `[bidi] [namespace ":"] name [bidi]`.
    ///
    /// Returns the name portion (without surrounding bidi controls) but
    /// advances pos past any trailing bidi controls.
    fn parse_identifier(&mut self) -> Option<&'a str> {
        let start = self.pos;
        // Skip leading bidi controls (ABNF: [bidi])
        self.skip_bidi_controls();
        let name_start = self.pos;
        // Try namespace:name
        let _first = self.parse_name()?;
        if self.eat(':') {
            // Must have a name after the colon
            if self.parse_name().is_none() {
                // Backtrack — the colon was not part of a namespace
                self.pos = start;
                self.skip_bidi_controls();
                let ns = self.pos;
                self.parse_name();
                let name_end = self.pos;
                self.skip_bidi_controls();
                return Some(&self.src[ns..name_end]);
            }
        }
        let name_end = self.pos;
        // Skip trailing bidi controls (ABNF: [bidi])
        self.skip_bidi_controls();
        if name_end == name_start {
            return None;
        }
        Some(&self.src[name_start..name_end])
    }

    // ─── Literal parsing ─────────────────────────────────────────────

    /// Parse ABNF `quoted-literal`: `"|" *(quoted-char / escape) "|"`.
    /// Returns the span *including* the pipe delimiters.
    fn parse_quoted_literal(&mut self) -> Option<&'a str> {
        let start = self.pos;
        if !self.eat('|') {
            return None;
        }
        loop {
            match self.peek() {
                Some('\\') => {
                    self.advance(); // backslash
                    self.advance(); // escaped char
                }
                Some('|') => {
                    self.advance();
                    return Some(&self.src[start..self.pos]);
                }
                Some(ch) if is_quoted_char(ch) || is_bidi_control(ch) => {
                    self.advance();
                }
                _ => {
                    // Unterminated — return what we have
                    return Some(&self.src[start..self.pos]);
                }
            }
        }
    }

    /// Parse ABNF `unquoted-literal`: `name / number-literal`.
    /// Also accepts bare `+` as a valid unquoted literal (MF2 spec).
    fn parse_unquoted_literal(&mut self) -> Option<&'a str> {
        let start = self.pos;
        // Try name first (strict ABNF `name` production)
        if let Some(name) = self.parse_name() {
            // Verify nothing token-like follows (e.g., name prefix of a broader token)
            if !self.is_unquoted_token_char() {
                return Some(name);
            }
            self.pos = start;
        } else {
            self.pos = start;
        }
        // Try number literal (strict ABNF `number-literal` production)
        if let Some(num) = self.try_parse_number_literal() {
            // Verify nothing token-like follows (e.g., `1.0e2` prefix of `1.0e2.0`)
            if !self.is_unquoted_token_char() {
                return Some(num);
            }
            self.pos = start;
        } else {
            self.pos = start;
        }
        // Fallback: accept any contiguous sequence of non-delimiter characters.
        // This handles tokens like `00`, `1.`, `1e+`, `-`, `·` etc. that are
        // neither valid names nor valid number-literals but should still parse
        // as unquoted literals (producing runtime errors, not syntax errors).
        self.parse_unquoted_token()
    }

    /// Check if the current character would be part of an unquoted token.
    fn is_unquoted_token_char(&self) -> bool {
        match self.peek() {
            None => false,
            Some(ch) => {
                !is_mf2_whitespace(ch)
                    && !matches!(ch, '{' | '}' | '|' | '\\' | '@' | ':' | '$' | '#' | '/')
                    && !is_bidi_control(ch)
            }
        }
    }

    /// Parse a broad unquoted token: any contiguous sequence of characters that
    /// are not expression delimiters (whitespace, `{`, `}`, `|`, `\`, `@`, `:`,
    /// `$`, `#`, `/`).
    fn parse_unquoted_token(&mut self) -> Option<&'a str> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if is_mf2_whitespace(ch)
                || matches!(ch, '{' | '}' | '|' | '\\' | '@' | ':' | '$' | '#' | '/')
                || is_bidi_control(ch)
            {
                break;
            }
            self.pos += ch.len_utf8();
        }
        if self.pos == start {
            return None;
        }
        Some(&self.src[start..self.pos])
    }

    /// Parse ABNF `literal`: `quoted-literal / unquoted-literal`.
    fn parse_literal(&mut self) -> Option<&'a str> {
        if self.peek() == Some('|') {
            self.parse_quoted_literal()
        } else {
            self.parse_unquoted_literal()
        }
    }

    /// Try to parse a number literal per MF2 rules.
    fn try_parse_number_literal(&mut self) -> Option<&'a str> {
        let start = self.pos;
        // Optional minus
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        // Integer part
        match self.peek() {
            Some('0') => {
                self.pos += 1;
            }
            Some(ch) if ch.is_ascii_digit() => {
                self.pos += 1;
                while let Some(ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => {
                self.pos = start;
                return None;
            }
        }
        // Fractional part
        if self.peek() == Some('.') {
            self.pos += 1;
            let frac_start = self.pos;
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == frac_start {
                self.pos = start;
                return None;
            }
        }
        // Exponent part
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == exp_start {
                self.pos = start;
                return None;
            }
        }
        if self.pos == start || (self.pos == start + 1 && self.src.as_bytes()[start] == b'-') {
            self.pos = start;
            return None;
        }
        // Ensure the next char is not a name-char (to avoid matching prefix of an identifier)
        if let Some(next) = self.peek() {
            if is_name_char(next) && next != '-' && next != '.' {
                self.pos = start;
                return None;
            }
        }
        Some(&self.src[start..self.pos])
    }

    /// Parse ABNF `variable`: `"$" [bidi] name [bidi]`.
    fn parse_variable(&mut self) -> Option<&'a str> {
        if !self.eat('$') {
            return None;
        }
        // Skip bidi controls between $ and name (ABNF: "$" [bidi] name)
        self.skip_bidi_controls();
        let name = self.parse_name()?;
        // Skip trailing bidi controls (ABNF: name [bidi])
        self.skip_bidi_controls();
        Some(name)
    }

    /// Skip bidi control characters (U+061C, U+200E, U+200F, U+2066-U+2069).
    fn skip_bidi_controls(&mut self) {
        while let Some(ch) = self.peek() {
            if is_bidi_control(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    // ─── Expression interior parsing ─────────────────────────────────

    /// Parse function annotation: `:identifier *(s option)`.
    fn parse_annotation(&mut self) -> Option<FunctionSpecNode<'a>> {
        if !self.eat(':') {
            return None;
        }
        let name_start = self.pos;
        let name = self.parse_identifier()?;
        let name_span = name_start..self.pos;

        let mut structured_options = Vec::new();
        let mut attributes = Vec::new();
        let mut flat_options = Vec::new();
        let mut flat_option_spans = Vec::new();

        loop {
            let saved = self.pos;
            if !self.skip_whitespace() {
                break;
            }
            if self.peek() == Some('@') {
                if let Some(attr) = self.parse_attribute() {
                    attributes.push(attr);
                    continue;
                }
            }
            if let Some(opt) = self.try_parse_option() {
                // Also build flat representation for backward compat
                let flat = &self.src[opt.span.clone()];
                flat_options.push(flat);
                flat_option_spans.push(opt.span.clone());
                structured_options.push(opt);
                continue;
            }
            // Try to capture a bare identifier as a flat option token
            // (for backward-compat error reporting via semantic.rs)
            let bare_start = self.pos;
            if let Some(_ident) = self.parse_identifier() {
                // Bare identifier without `=` — capture as flat option for error reporting
                let bare_end = self.pos;
                let flat = &self.src[bare_start..bare_end];
                flat_options.push(flat);
                flat_option_spans.push(bare_start..bare_end);
                continue;
            }
            // Not an option or attribute — backtrack whitespace
            self.pos = saved;
            break;
        }

        Some(FunctionSpecNode {
            name,
            name_span,
            options: flat_options,
            option_spans: flat_option_spans,
            structured_options,
            attributes,
        })
    }

    /// Try to parse an option: `identifier [s] "=" [s] (literal / variable)`.
    fn try_parse_option(&mut self) -> Option<OptionNode<'a>> {
        let start = self.pos;
        let name = self.parse_identifier()?;
        self.skip_optional_whitespace();
        if !self.eat('=') {
            self.pos = start;
            return None;
        }
        self.skip_optional_whitespace();
        let value = self.parse_option_value()?;
        Some(OptionNode {
            name,
            value,
            span: start..self.pos,
        })
    }

    fn parse_option_value(&mut self) -> Option<OptionValue<'a>> {
        if self.peek() == Some('$') {
            let var_start = self.pos;
            let var = self.parse_variable()?;
            // Return the variable name (without $)
            let _ = var_start; // keep for potential span use
            Some(OptionValue::Variable(var))
        } else {
            let lit = self.parse_literal()?;
            Some(OptionValue::Literal(lit))
        }
    }

    /// Parse attribute: `"@" identifier [[s] "=" [s] literal]`.
    ///
    /// Per ABNF, attribute values are literals only (not variables).
    fn parse_attribute(&mut self) -> Option<AttributeNode<'a>> {
        let start = self.pos;
        if !self.eat('@') {
            return None;
        }
        let name = self.parse_identifier()?;
        let saved = self.pos;
        self.skip_optional_whitespace();
        let value = if self.eat('=') {
            self.skip_optional_whitespace();
            let lit = self.parse_literal()?;
            Some(OptionValue::Literal(lit))
        } else {
            self.pos = saved;
            None
        };
        Some(AttributeNode {
            name,
            value,
            span: start..self.pos,
        })
    }

    /// Parse markup: `"#" identifier ...` or `"/" identifier ...`.
    /// Called after `{` and optional whitespace when `#` or `/` is seen.
    fn parse_markup(&mut self) -> Option<ExpressionPayloadNode<'a>> {
        let start = self.pos;
        let is_close = self.eat('/');
        let is_open = !is_close && self.eat('#');
        if !is_close && !is_open {
            return None;
        }
        let identifier = self.parse_identifier()?;

        let mut options = Vec::new();
        let mut attributes = Vec::new();

        loop {
            let saved = self.pos;
            if !self.skip_whitespace() {
                break;
            }
            if self.peek() == Some('@') {
                if let Some(attr) = self.parse_attribute() {
                    attributes.push(attr);
                    continue;
                }
            }
            if let Some(opt) = self.try_parse_option() {
                options.push(opt);
                continue;
            }
            self.pos = saved;
            break;
        }

        self.skip_optional_whitespace();
        let kind = if is_close {
            MarkupKind::Close
        } else if self.peek() == Some('/') {
            self.advance();
            MarkupKind::SelfClose
        } else {
            MarkupKind::Open
        };

        Some(ExpressionPayloadNode::Markup(MarkupNode {
            kind,
            identifier,
            options,
            attributes,
            span: start..self.pos,
        }))
    }

    /// Parse the interior of a `{ ... }` expression (after `{`, before `}`).
    fn parse_expression_interior(&mut self) -> ExpressionNode<'a> {
        let outer_start = self.pos - 1; // include the `{`
        self.skip_optional_whitespace();
        let inner_start = self.pos;

        // Check for markup: `#identifier` or `/identifier`
        if matches!(self.peek(), Some('#' | '/')) {
            if let Some(markup_payload) = self.parse_markup() {
                self.skip_optional_whitespace();
                let inner_end = self.pos;
                // Expect `}`
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Literal,

                    payload: Some(markup_payload),
                    diag_hint: None,
                };
            }
        }

        // Check for reserved/private-use annotation sigils (not bare `+`)
        if let Some(ch) = self.peek() {
            if RESERVED_SIGILS.contains(&ch)
                || (ch == '+'
                    && self
                        .peek_nth(1)
                        .is_some_and(|c| c != '}' && !is_ignorable_char(c)))
            {
                // Skip past the reserved annotation body
                self.skip_to_expression_end();
                let inner_end = self.pos;
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Literal,

                    payload: None,
                    diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                };
            }
        }

        // Try variable
        if self.peek() == Some('$') {
            let var_start = self.pos;
            // Try strict variable parse first
            let var_result = self.parse_variable();
            // If strict parse fails, try to consume any raw token after $ for error recovery
            let (var_name, var_end) = if let Some(name) = var_result {
                (name, self.pos)
            } else {
                // Recover: consume chars until whitespace, `:`, `}`, or `@`
                self.pos = var_start + 1; // skip past $
                let name_start = self.pos;
                while let Some(ch) = self.peek() {
                    if is_ignorable_char(ch) || matches!(ch, ':' | '}' | '@') {
                        break;
                    }
                    self.pos += ch.len_utf8();
                }
                if self.pos == name_start {
                    // Nothing after $, produce error node
                    let inner_end = self.pos;
                    self.skip_to_expression_end();
                    self.eat('}');
                    return ExpressionNode {
                        raw_span: outer_start..self.pos,
                        span: inner_start..inner_end,
                        kind: ExpressionKindNode::Var,

                        payload: Some(ExpressionPayloadNode::Var(VarExpressionNode {
                            name_span: name_start..name_start,
                            name: "",
                        })),
                        diag_hint: None,
                    };
                }
                (&self.src[name_start..self.pos], self.pos)
            };
            {
                let had_ws_after_var = self.skip_whitespace();

                // Check for annotation `:function`
                if self.peek() == Some(':') {
                    if let Some(function) = self.parse_annotation() {
                        self.skip_optional_whitespace();
                        // Consume trailing attributes at expression level
                        while self.peek() == Some('@') {
                            if let Some(attr) = self.parse_attribute() {
                                let _ = attr;
                                self.skip_optional_whitespace();
                            } else {
                                break;
                            }
                        }

                        // After annotation + attributes, validate next is `}`
                        self.skip_optional_whitespace();
                        if self.peek() != Some('}') && !self.at_end() {
                            self.skip_to_expression_end();
                            let inner_end = self.pos;
                            self.eat('}');
                            return ExpressionNode {
                                raw_span: outer_start..self.pos,
                                span: inner_start..inner_end,
                                kind: ExpressionKindNode::Call,
                                payload: None,
                                diag_hint: Some(
                                    ExpressionDiagnosticHint::NonSelectPayloadUnavailable,
                                ),
                            };
                        }

                        let inner_end = self.pos;
                        self.eat('}');
                        return ExpressionNode {
                            raw_span: outer_start..self.pos,
                            span: inner_start..inner_end,
                            kind: ExpressionKindNode::Call,

                            payload: Some(ExpressionPayloadNode::Call(CallExpressionNode {
                                operand: CallOperandNode::Var(VarExpressionNode {
                                    name_span: (var_start + 1)..var_end,
                                    name: var_name,
                                }),
                                function,
                            })),
                            diag_hint: None,
                        };
                    }
                    // Colon but no valid function name
                    self.skip_to_expression_end();
                    let inner_end = self.pos;
                    self.eat('}');
                    return ExpressionNode {
                        raw_span: outer_start..self.pos,
                        span: inner_start..inner_end,
                        kind: ExpressionKindNode::Call,

                        payload: None,
                        diag_hint: Some(ExpressionDiagnosticHint::FunctionNameRequiredAfterColon),
                    };
                }

                // Check for reserved/private-use annotation after variable
                if let Some(ch) = self.peek() {
                    if is_reserved_or_private_use_start_char(ch)
                        || (ch == '+'
                            && self
                                .peek_nth(1)
                                .is_some_and(|c| c != '}' && !is_ignorable_char(c)))
                    {
                        self.skip_to_expression_end();
                        let inner_end = self.pos;
                        self.eat('}');
                        return ExpressionNode {
                            raw_span: outer_start..self.pos,
                            span: inner_start..inner_end,
                            kind: ExpressionKindNode::Literal,

                            payload: None,
                            diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                        };
                    }
                }

                // Consume trailing attributes (ABNF: *(s attribute) — requires preceding space)
                if had_ws_after_var && self.peek() == Some('@') {
                    while self.peek() == Some('@') {
                        let saved = self.pos;
                        if self.parse_attribute().is_none() {
                            self.pos = saved;
                            break;
                        }
                        self.skip_optional_whitespace();
                    }
                }

                // After operand + annotation + attributes, next must be `}`
                self.skip_optional_whitespace();
                if self.peek() != Some('}') && !self.at_end() {
                    // Junk after operand — syntax error
                    self.skip_to_expression_end();
                    let inner_end = self.pos;
                    self.eat('}');
                    return ExpressionNode {
                        raw_span: outer_start..self.pos,
                        span: inner_start..inner_end,
                        kind: ExpressionKindNode::Var,

                        payload: None,
                        diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                    };
                }

                let inner_end = self.pos;
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Var,

                    payload: Some(ExpressionPayloadNode::Var(VarExpressionNode {
                        name_span: (var_start + 1)..var_end,
                        name: var_name,
                    })),
                    diag_hint: None,
                };
            }
        }

        // Try literal (quoted or unquoted)
        let literal_start = self.pos;
        if let Some(literal_value) = self.parse_literal() {
            let literal_end = self.pos;
            let had_ws_after_literal = self.skip_whitespace();

            // Check for annotation (`:function`) — requires space after literal
            if self.peek() == Some(':') {
                if let Some(function) = self.parse_annotation() {
                    self.skip_optional_whitespace();
                    // Consume trailing attributes (must be preceded by whitespace)
                    while self.peek() == Some('@') {
                        let saved = self.pos;
                        if self.parse_attribute().is_none() {
                            self.pos = saved;
                            break;
                        }
                        self.skip_optional_whitespace();
                    }
                    // After annotation + attributes, validate next is `}`
                    self.skip_optional_whitespace();
                    if self.peek() != Some('}') && !self.at_end() {
                        self.skip_to_expression_end();
                        let inner_end = self.pos;
                        self.eat('}');
                        return ExpressionNode {
                            raw_span: outer_start..self.pos,
                            span: inner_start..inner_end,
                            kind: ExpressionKindNode::Literal,

                            payload: None,
                            diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                        };
                    }
                    let inner_end = self.pos;
                    self.eat('}');
                    return ExpressionNode {
                        raw_span: outer_start..self.pos,
                        span: inner_start..inner_end,
                        kind: ExpressionKindNode::Literal,

                        payload: Some(ExpressionPayloadNode::Literal(LiteralExpressionNode {
                            value_span: literal_start..literal_end,
                            value: literal_value,
                            function: Some(function),
                            is_markup: false,
                        })),
                        diag_hint: None,
                    };
                }
                // Colon but no valid function
                self.skip_to_expression_end();
                let inner_end = self.pos;
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Literal,

                    payload: None,
                    diag_hint: Some(ExpressionDiagnosticHint::FunctionNameRequiredAfterColon),
                };
            }

            // Check for reserved/private-use annotation after literal
            if let Some(ch) = self.peek() {
                if is_reserved_or_private_use_start_char(ch)
                    || (ch == '+'
                        && self
                            .peek_nth(1)
                            .is_some_and(|c| c != '}' && !is_ignorable_char(c)))
                {
                    self.skip_to_expression_end();
                    let inner_end = self.pos;
                    self.eat('}');
                    return ExpressionNode {
                        raw_span: outer_start..self.pos,
                        span: inner_start..inner_end,
                        kind: ExpressionKindNode::Literal,

                        payload: None,
                        diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                    };
                }
            }

            // Consume trailing attributes (ABNF: *(s attribute) — requires preceding space)
            if had_ws_after_literal && self.peek() == Some('@') {
                while self.peek() == Some('@') {
                    let saved = self.pos;
                    if self.parse_attribute().is_none() {
                        self.pos = saved;
                        break;
                    }
                    self.skip_optional_whitespace();
                }
            }

            // After operand + optional attributes, next must be `}`
            self.skip_optional_whitespace();
            if self.peek() != Some('}') && !self.at_end() {
                // Junk after literal — syntax error
                self.skip_to_expression_end();
                let inner_end = self.pos;
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Literal,

                    payload: None,
                    diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
                };
            }

            let inner_end = self.pos;
            self.eat('}');
            return ExpressionNode {
                raw_span: outer_start..self.pos,
                span: inner_start..inner_end,
                kind: ExpressionKindNode::Literal,

                payload: Some(ExpressionPayloadNode::Literal(LiteralExpressionNode {
                    value_span: literal_start..literal_end,
                    value: literal_value,
                    function: None,
                    is_markup: false,
                })),
                diag_hint: None,
            };
        }

        // Bare annotation (no operand): `:function`
        if self.peek() == Some(':') {
            if let Some(function) = self.parse_annotation() {
                self.skip_optional_whitespace();
                let inner_end = self.pos;
                self.eat('}');
                return ExpressionNode {
                    raw_span: outer_start..self.pos,
                    span: inner_start..inner_end,
                    kind: ExpressionKindNode::Literal,

                    payload: Some(ExpressionPayloadNode::Literal(LiteralExpressionNode {
                        value_span: inner_start..inner_start,
                        value: "",
                        function: Some(function),
                        is_markup: false,
                    })),
                    diag_hint: None,
                };
            }
        }

        // Nothing recognized — consume to `}` and return error node
        self.skip_to_expression_end();
        let inner_end = self.pos;
        self.eat('}');
        ExpressionNode {
            raw_span: outer_start..self.pos,
            span: inner_start..inner_end,
            kind: ExpressionKindNode::Literal,

            payload: None,
            diag_hint: Some(ExpressionDiagnosticHint::NonSelectPayloadUnavailable),
        }
    }

    fn skip_to_expression_end(&mut self) {
        let mut depth = 0_i32;
        while let Some(ch) = self.peek() {
            match ch {
                '{' => {
                    depth += 1;
                    self.advance();
                }
                '}' => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.advance();
                }
                '\\' => {
                    self.advance();
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ─── Pattern parsing ─────────────────────────────────────────────

    /// Build a pattern node from a byte range, extracting text and expression segments.
    fn build_pattern_node(&mut self, start: usize, end: usize) -> PatternNode<'a> {
        let saved = self.pos;
        self.pos = start;
        let mut segments = Vec::new();

        while self.pos < end {
            // Scan for text until `{` or end
            let text_start = self.pos;
            let mut escaped = false;
            let mut found_open = false;
            while self.pos < end {
                let Some(ch) = self.peek() else { break };
                if escaped {
                    escaped = false;
                    self.pos += ch.len_utf8();
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    self.pos += ch.len_utf8();
                    continue;
                }
                if ch == '{' {
                    found_open = true;
                    break;
                }
                self.pos += ch.len_utf8();
            }

            if text_start < self.pos {
                segments.push(PatternSegmentNode::Text {
                    span: text_start..self.pos,
                    value: &self.src[text_start..self.pos],
                });
            }

            if !found_open {
                break;
            }

            // We're at `{` — find the expression
            self.advance(); // eat `{`
            let expr = self.parse_expression_interior();
            segments.push(PatternSegmentNode::Expression(Box::new(expr)));
        }

        self.pos = saved;
        PatternNode {
            span: start..end,
            segments,
        }
    }

    // ─── Declaration parsing ─────────────────────────────────────────

    fn parse_declaration_head(&mut self) -> Option<(DeclarationKind, usize)> {
        if !self.eat('.') {
            return None;
        }
        let name_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == name_start {
            return None;
        }
        let name = &self.src[name_start..self.pos];
        let kind = match name {
            "input" => DeclarationKind::Input,
            "local" => DeclarationKind::Local,
            "match" => DeclarationKind::Match,
            _ => return None,
        };
        Some((kind, self.pos))
    }

    fn parse_input_payload(&mut self) -> Option<DeclarationPayloadNode<'a>> {
        self.skip_optional_whitespace();
        let open = self.pos;
        if !self.eat('{') {
            return None;
        }
        // Verify the opening brace has a matching close before parsing interior.
        if find_matching_brace(self.src, open).is_none() {
            self.pos = self.src.len();
            return None;
        }
        let expr = self.parse_expression_interior();
        Some(DeclarationPayloadNode::Input { expr })
    }

    fn parse_local_payload(&mut self) -> Option<DeclarationPayloadNode<'a>> {
        if !self.skip_whitespace() {
            return None;
        }
        if !self.eat('$') {
            return None;
        }
        // Skip bidi controls between $ and name (ABNF: "$" [bidi] name [bidi])
        self.skip_bidi_controls();
        let name_start = self.pos;
        let name = self.parse_name()?;
        let name_end = self.pos;
        self.skip_bidi_controls();
        self.skip_optional_whitespace();
        if !self.eat('=') {
            return None;
        }
        self.skip_optional_whitespace();
        let open = self.pos;
        if !self.eat('{') {
            return None;
        }
        // Verify the opening brace has a matching close before parsing interior.
        if find_matching_brace(self.src, open).is_none() {
            self.pos = self.src.len();
            return None;
        }
        let expr = self.parse_expression_interior();
        Some(DeclarationPayloadNode::Local {
            name_span: name_start..name_end,
            name,
            expr,
        })
    }

    fn parse_match_payload(
        &mut self,
    ) -> (usize, Result<DeclarationPayloadNode<'a>, MatchParseError>) {
        // Must have whitespace after `.match`
        if !self.skip_whitespace() {
            // Disambiguate: peek the char at `self.pos` (skip_whitespace
            // backtracks any bidi it tentatively consumed on failure).
            // - None or a bidi control → MissingSelector (matches the
            //   old semantic heuristic which treated bidi as ignorable
            //   and fell through to the selector diagnostic).
            // - Anything else → MissingWhitespaceAfterMatch (preserves
            //   the byte-for-byte diagnostic from semantic.rs).
            let after_match = self.pos;
            match self.peek() {
                None => {
                    return (
                        self.pos,
                        Err(MatchParseError::MissingSelector { pos: after_match }),
                    );
                }
                Some(ch) if is_bidi_control(ch) => {
                    return (
                        self.pos,
                        Err(MatchParseError::MissingSelector { pos: after_match }),
                    );
                }
                Some(_) => {
                    return (
                        self.pos,
                        Err(MatchParseError::MissingWhitespaceAfterMatch { after_match }),
                    );
                }
            }
        }

        // Parse selectors: `1*([s] selector)` where selector = variable
        let mut selectors = Vec::new();
        let mut had_trailing_ws = false;
        while self.peek() == Some('$') {
            let var_full_start = self.pos;
            self.advance(); // eat $
            let name_start = self.pos;
            if let Some(_name) = self.parse_name() {
                let name_end = self.pos;
                let span = trim_ignorable_span(self.src, name_start, name_end);
                selectors.push(ExpressionNode {
                    raw_span: span.clone(),
                    span: span.clone(),
                    kind: ExpressionKindNode::Var,

                    payload: Some(ExpressionPayloadNode::Var(VarExpressionNode {
                        name: self.src.get(span.clone()).unwrap_or_default(),
                        name_span: span,
                    })),
                    diag_hint: None,
                });
                had_trailing_ws = self.skip_actual_whitespace();
                if !had_trailing_ws {
                    break;
                }
            } else {
                self.pos = var_full_start;
                break;
            }
        }

        if selectors.is_empty() {
            return (
                self.pos,
                Err(MatchParseError::MissingSelector { pos: self.pos }),
            );
        }

        // Require whitespace between last selector and variant keys
        // (ABNF: selectors and variants are separated by `s`)
        if !had_trailing_ws {
            return (
                self.pos,
                Err(MatchParseError::MissingWhitespaceAfterSelector { pos: self.pos }),
            );
        }

        // Parse variants: greedy key parsing until `{{`
        let mut variants = Vec::new();
        let mut last_key_span: core::ops::Range<usize> = 0..0;
        loop {
            self.skip_optional_whitespace();
            if self.at_end() {
                break;
            }
            // Check if we're at a variant start (key or `{{`)
            if self.remaining().starts_with("{{") {
                // This would be a variant with no keys — shouldn't happen
                break;
            }

            // Parse keys greedily until `{{`
            // ABNF: key *(s key) — mandatory whitespace between keys
            let mut keys = Vec::new();
            loop {
                if !keys.is_empty() {
                    // After the first key, require whitespace before next key
                    if !self.skip_whitespace() {
                        break;
                    }
                } else {
                    self.skip_optional_whitespace();
                }
                if self.at_end() || self.remaining().starts_with("{{") {
                    break;
                }
                let key_start = self.pos;
                let Some((key, next)) = self.parse_match_key() else {
                    return (
                        self.pos,
                        Err(MatchParseError::MalformedKey {
                            span: key_start..self.pos,
                        }),
                    );
                };
                last_key_span = key.span.clone();
                keys.push(key);
                self.pos = next;
            }

            if keys.is_empty() {
                break;
            }

            // Parse variant pattern `{{...}}`
            self.skip_optional_whitespace();
            let pattern_start = self.pos;
            let Some((pattern_span, next)) = self.parse_variant_pattern_span() else {
                let err = if self.pos == pattern_start {
                    MatchParseError::MissingVariantPattern {
                        last_key_span: last_key_span.clone(),
                        expected_at: pattern_start,
                    }
                } else {
                    MatchParseError::UnterminatedVariantPattern {
                        last_key_span: last_key_span.clone(),
                        pattern_start,
                    }
                };
                return (self.pos, Err(err));
            };
            let pattern = self.build_pattern_node(pattern_span.start, pattern_span.end);
            variants.push(MatchVariantNode { keys, pattern });
            self.pos = next;
        }

        if variants.is_empty() {
            return (self.pos, Err(MatchParseError::NoVariants { pos: self.pos }));
        }

        (
            self.pos,
            Ok(DeclarationPayloadNode::Match {
                selectors,
                variants,
            }),
        )
    }

    fn parse_match_key(&mut self) -> Option<(VariantKeyNode<'a>, usize)> {
        if self.eat('*') {
            let span = (self.pos - 1)..self.pos;
            return Some((
                VariantKeyNode {
                    span,
                    text: "*",
                    is_default: true,
                    is_quoted: false,
                },
                self.pos,
            ));
        }

        if self.peek() == Some('|') {
            let _start = self.pos;
            self.advance(); // eat `|`
            let content_start = self.pos;
            loop {
                match self.peek() {
                    Some('\\') => {
                        self.advance();
                        self.advance(); // skip escaped char
                    }
                    Some('|') => {
                        let content_end = self.pos;
                        self.advance(); // eat closing `|`
                        return Some((
                            VariantKeyNode {
                                span: content_start..content_end,
                                text: &self.src[content_start..content_end],
                                is_default: false,
                                is_quoted: true,
                            },
                            self.pos,
                        ));
                    }
                    Some(_) => {
                        self.advance();
                    }
                    None => return None,
                }
            }
        }

        // Unquoted key — read until whitespace or `{`
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if is_ignorable_char(ch) || ch == '{' {
                break;
            }
            self.pos += ch.len_utf8();
        }
        if self.pos == start {
            return None;
        }
        Some((
            VariantKeyNode {
                span: start..self.pos,
                text: &self.src[start..self.pos],
                is_default: false,
                is_quoted: false,
            },
            self.pos,
        ))
    }

    fn parse_variant_pattern_span(&mut self) -> Option<(core::ops::Range<usize>, usize)> {
        if !self.eat_str("{{") {
            return None;
        }
        let start = self.pos;
        let mut depth = 1_i32;
        let mut single_brace_depth = 0_i32;
        while self.pos < self.src.len() {
            if self.remaining().starts_with("{{") {
                depth += 1;
                self.pos += 2;
                continue;
            }
            if single_brace_depth == 0 && self.remaining().starts_with("}}") {
                depth -= 1;
                if depth == 0 {
                    let end = self.pos;
                    self.pos += 2;
                    return Some((start..end, self.pos));
                }
                self.pos += 2;
                continue;
            }
            let ch = self.peek()?;
            if ch == '{' {
                single_brace_depth += 1;
            } else if ch == '}' && single_brace_depth > 0 {
                single_brace_depth -= 1;
            }
            self.pos += ch.len_utf8();
        }
        None
    }
}

// ─── Public API (preserving existing signatures) ─────────────────────────────

/// Parse a source string into syntax document scaffolding.
pub(crate) fn parse_document(source: &str) -> SyntaxDocument<'_> {
    let mut parser = Parser::new(source);
    let mut declarations = Vec::new();

    loop {
        parser.skip_optional_whitespace();
        if parser.at_end() || parser.peek() != Some('.') {
            break;
        }

        let start = parser.pos;
        let saved = parser.pos;

        let Some((kind, _cursor)) = parser.parse_declaration_head() else {
            parser.pos = saved;
            break;
        };

        match kind {
            DeclarationKind::Input => {
                if let Some(payload) = parser.parse_input_payload() {
                    declarations.push(DeclarationNode {
                        kind,
                        span: start..parser.pos,
                        payload: Some(payload),
                        match_error: None,
                    });
                } else {
                    // Malformed — record declaration without payload
                    declarations.push(DeclarationNode {
                        kind,
                        span: start..parser.pos,
                        payload: None,
                        match_error: None,
                    });
                    // Try to recover by skipping to next declaration
                    break;
                }
            }
            DeclarationKind::Local => {
                if let Some(payload) = parser.parse_local_payload() {
                    declarations.push(DeclarationNode {
                        kind,
                        span: start..parser.pos,
                        payload: Some(payload),
                        match_error: None,
                    });
                } else {
                    declarations.push(DeclarationNode {
                        kind,
                        span: start..parser.pos,
                        payload: None,
                        match_error: None,
                    });
                    break;
                }
            }
            DeclarationKind::Match => {
                let (end, payload_result) = parser.parse_match_payload();
                let (payload, match_error) = match payload_result {
                    Ok(p) => (Some(p), None),
                    Err(e) => (None, Some(e)),
                };
                declarations.push(DeclarationNode {
                    kind,
                    span: start..end,
                    payload,
                    match_error,
                });
                // Match consumes everything
                parser.pos = end;
            }
        }
    }

    parser.skip_optional_whitespace();
    let body = if parser.pos < source.len() {
        Some(parser.build_pattern_node(parser.pos, source.len()))
    } else {
        None
    };

    SyntaxDocument { declarations, body }
}

/// Parse a source string as a pattern body node.
pub(crate) fn parse_pattern(source: &str) -> PatternNode<'_> {
    let mut parser = Parser::new(source);
    parser.build_pattern_node(0, source.len())
}

/// Validate declaration heads in complex-message source.
///
/// Per the MF2 ABNF, `.` is a valid `text-char` but **not** a valid
/// `simple-start-char` — so a leading `.` (after optional whitespace / bidi
/// isolates) is what disambiguates a complex-message declaration prelude
/// from a simple-message. Anywhere else, `.` is just text.
///
/// This validator delegates to `parse_document`: if the residual body starts
/// with `.`, we require the following ASCII-alphabetic run to name a known
/// declaration keyword (`input` / `local` / `match`). Empty alpha run
/// (`.`, `. foo`, `.1`, `.{`, …) and unknown alpha run (`.bogus`, `.inupt`,
/// …) both fail here.
pub(crate) fn validate_known_declaration_heads(
    source: &str,
    ctx: crate::syntax::span::SourceContext,
) -> Result<(), CompileError> {
    let doc = parse_document(source);
    let Some(body) = &doc.body else {
        return Ok(());
    };
    let body_start = body.span.start;
    let body_src = &source[body_start..];
    if !body_src.starts_with('.') {
        return Ok(());
    }
    let bytes = body_src.as_bytes();
    let mut end = 1;
    while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
        end += 1;
    }
    let head = &body_src[1..end];
    if end > 1 && (head == "input" || head == "local" || head == "match") {
        return Ok(());
    }
    let (line, col) = ctx.location(source, body_start);
    Err(CompileError::invalid_expr_detail(
        line,
        col,
        ".input, .local, or .match",
        crate::syntax::span::quoted_snippet(&body_src[..end]),
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// TR35 reserved/private-use annotation sigils.
const RESERVED_SIGILS: &[char] = &['!', '%', '^', '&', '*', '<', '>', '?', '~'];

fn find_matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = open;
    let mut depth = 0_i32;
    let mut in_quoted = false;
    while cursor < bytes.len() {
        if in_quoted {
            if bytes[cursor] == b'\\' {
                cursor = cursor.saturating_add(2);
                continue;
            }
            if bytes[cursor] == b'|' {
                in_quoted = false;
            }
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'|' {
            in_quoted = true;
        } else if bytes[cursor] == b'{' {
            depth += 1;
        } else if bytes[cursor] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

fn is_reserved_or_private_use_start_char(ch: char) -> bool {
    RESERVED_SIGILS.contains(&ch) || ch == '+'
}

fn is_ignorable_char(ch: char) -> bool {
    is_mf2_whitespace(ch) || is_bidi_control(ch)
}

fn trim_ignorable_span(source: &str, mut start: usize, mut end: usize) -> core::ops::Range<usize> {
    while start < end {
        let Some(ch) = source[start..].chars().next() else {
            break;
        };
        if is_ignorable_char(ch) {
            start += ch.len_utf8();
        } else {
            break;
        }
    }
    while start < end {
        let Some(ch) = source[..end].chars().next_back() else {
            break;
        };
        if is_ignorable_char(ch) {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::{find_matching_brace, parse_document, validate_known_declaration_heads};
    use crate::syntax::ast::{
        DeclarationKind, DeclarationPayloadNode, ExpressionKindNode, ExpressionNode,
        ExpressionPayloadNode, PatternNode, PatternSegmentNode,
    };

    fn expression_segments<'a>(pattern: &'a PatternNode<'a>) -> Vec<&'a ExpressionNode<'a>> {
        pattern
            .segments
            .iter()
            .filter_map(|segment| match segment {
                PatternSegmentNode::Expression(expr) => Some(expr.as_ref()),
                PatternSegmentNode::Text { .. } => None,
            })
            .collect()
    }

    #[test]
    fn parses_declaration_prelude_segments() {
        let src = ".input {$x :number} .local $y = {$x} {{Hello {$y}}}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 2);
        assert_eq!(doc.declarations[0].kind, DeclarationKind::Input);
        assert_eq!(doc.declarations[1].kind, DeclarationKind::Local);
        assert!(doc.declarations[0].payload.is_some());
        assert!(doc.declarations[1].payload.is_some());
        let Some(DeclarationPayloadNode::Input { expr }) = &doc.declarations[0].payload else {
            panic!("expected input payload");
        };
        assert_eq!(expr.kind, ExpressionKindNode::Call);
        let Some(DeclarationPayloadNode::Local { expr, .. }) = &doc.declarations[1].payload else {
            panic!("expected local payload");
        };
        assert_eq!(expr.kind, ExpressionKindNode::Var);
        assert!(doc.body.is_some_and(|it| src[it.span].contains("Hello")));
    }

    #[test]
    fn parses_match_declaration_payload() {
        let src = ".match $n one {{One}} * {{Other}}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 1);
        assert_eq!(doc.declarations[0].kind, DeclarationKind::Match);
        let Some(DeclarationPayloadNode::Match {
            selectors,
            variants,
        }) = &doc.declarations[0].payload
        else {
            panic!("expected match payload");
        };
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].kind, ExpressionKindNode::Var);
        assert_eq!(&src[selectors[0].span.clone()], "n");
        assert_eq!(variants.len(), 2);
        assert_eq!(&src[variants[0].pattern.span.clone()], "One");
        assert_eq!(&src[variants[1].pattern.span.clone()], "Other");
    }

    #[test]
    fn parses_match_quoted_keys_with_escapes() {
        let src = ".match $n |a\\|b| {{Hit}} * {{Other}}";
        let doc = parse_document(src);
        let Some(DeclarationPayloadNode::Match { variants, .. }) = &doc.declarations[0].payload
        else {
            panic!("expected match payload");
        };
        assert!(variants[0].keys[0].is_quoted);
        assert_eq!(&src[variants[0].keys[0].span.clone()], "a\\|b");
    }

    #[test]
    fn parses_match_nested_variant_pattern_braces() {
        let src = ".match $n one {{{{inner}}}} * {{Other}}";
        let doc = parse_document(src);
        let Some(DeclarationPayloadNode::Match { variants, .. }) = &doc.declarations[0].payload
        else {
            panic!("expected match payload");
        };
        assert_eq!(&src[variants[0].pattern.span.clone()], "{{inner}}");
        assert_eq!(&src[variants[1].pattern.span.clone()], "Other");
    }

    #[test]
    fn parses_match_pattern_containing_expression_braces() {
        let src = ".match $n one {{literal {$n}}} * {{Other}}";
        let doc = parse_document(src);
        let Some(DeclarationPayloadNode::Match { variants, .. }) = &doc.declarations[0].payload
        else {
            panic!("expected match payload");
        };
        assert_eq!(&src[variants[0].pattern.span.clone()], "literal {$n}");
        assert_eq!(&src[variants[1].pattern.span.clone()], "Other");
        let exprs = expression_segments(&variants[0].pattern);
        assert_eq!(exprs.len(), 1);
        assert_eq!(exprs[0].kind, ExpressionKindNode::Var);
    }

    #[test]
    fn malformed_input_payload_is_retained_as_invalid_declaration() {
        let src = ".input {$x .local $y = {1}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 1);
        assert_eq!(doc.declarations[0].kind, DeclarationKind::Input);
        // The payload may or may not be present depending on how the malformed
        // expression is recovered — the key invariant is that only one
        // declaration is produced.
    }

    #[test]
    fn malformed_local_payload_is_retained_as_invalid_declaration() {
        let src = ".local $x = {1 .input {$y}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 1);
        assert_eq!(doc.declarations[0].kind, DeclarationKind::Local);
    }

    #[test]
    fn pattern_body_collects_expression_spans() {
        let src = "hello = Hi { $name } and { $count :number }";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 0);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 2);
        assert_eq!(exprs[0].kind, ExpressionKindNode::Var);
        assert_eq!(exprs[1].kind, ExpressionKindNode::Call);
    }

    #[test]
    fn pattern_body_collects_text_and_expression_segments() {
        let src = "hello \\{not\\} { $name } tail";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        assert_eq!(body.segments.len(), 3);
        match &body.segments[0] {
            PatternSegmentNode::Text { span, value } => {
                assert_eq!(&src[span.clone()], "hello \\{not\\} ");
                assert_eq!(*value, "hello \\{not\\} ");
            }
            other => panic!("unexpected segment: {other:?}"),
        }
        match &body.segments[1] {
            PatternSegmentNode::Expression(expr) => {
                assert_eq!(expr.kind, ExpressionKindNode::Var);
            }
            other => panic!("unexpected segment: {other:?}"),
        }
        match &body.segments[2] {
            PatternSegmentNode::Text { span, value } => {
                assert_eq!(&src[span.clone()], " tail");
                assert_eq!(*value, " tail");
            }
            other => panic!("unexpected segment: {other:?}"),
        }
    }

    #[test]
    fn pattern_body_ignores_escaped_open_braces() {
        let src = "hello \\{not-expr\\} { $name }";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert_eq!(exprs[0].kind, ExpressionKindNode::Var);
    }

    #[test]
    fn pattern_body_non_select_expressions_have_typed_payloads() {
        let src = "{ $x } { $n :number minimumFractionDigits=2 } { 42 :integer }";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 3);
        assert!(matches!(
            exprs[0].payload,
            Some(ExpressionPayloadNode::Var(_))
        ));
        assert!(matches!(
            exprs[1].payload,
            Some(ExpressionPayloadNode::Call(_))
        ));
        assert!(matches!(
            exprs[2].payload,
            Some(ExpressionPayloadNode::Literal(_))
        ));
    }

    #[test]
    fn validate_known_declaration_heads_rejects_unknown_head() {
        let err = validate_known_declaration_heads(
            ".bogus {$x}",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidExpr {
                line,
                column,
                expected,
                found,
            } => {
                assert_eq!(line, 1);
                assert_eq!(column, 1);
                assert_eq!(expected, Some(".input, .local, or .match"));
                assert!(found.is_some());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn find_matching_brace_ignores_quoted_literal_braces() {
        let src = "{|a}b|}";
        assert_eq!(find_matching_brace(src, 0), Some(src.len() - 1));
    }

    #[test]
    fn validate_known_declaration_heads_accepts_ideographic_space_before_head() {
        validate_known_declaration_heads(
            "\u{3000}.match $n one {{One}} * {{Other}}",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect("valid declaration head");
    }

    #[test]
    fn validate_known_declaration_heads_accepts_dot_in_simple_message_text() {
        validate_known_declaration_heads(
            "Hello .world",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect("dot-prefixed word is valid text in a simple message");
    }

    #[test]
    fn validate_known_declaration_heads_accepts_dot_after_newline() {
        validate_known_declaration_heads(
            "Hello\n.foo",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect("dot after a newline is still text inside a simple message");
    }

    #[test]
    fn validate_known_declaration_heads_accepts_dot_in_quoted_pattern_body() {
        validate_known_declaration_heads(
            ".input {$x :string}\n{{Greetings .foo}}",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect("dot-prefixed word inside a quoted pattern is valid text");
    }

    #[test]
    fn validate_known_declaration_heads_rejects_mid_prelude_typo() {
        let err = validate_known_declaration_heads(
            ".input {$x} .inupt {$y}",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect_err("typo'd mid-prelude declaration must still be rejected");
        match err {
            crate::compile::CompileError::InvalidExpr {
                expected, found, ..
            } => {
                assert_eq!(expected, Some(".input, .local, or .match"));
                assert!(found.is_some());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_known_declaration_heads_rejects_leading_dot_without_alpha() {
        validate_known_declaration_heads(
            ". foo",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect_err("leading `.` not followed by an alpha keyword is invalid");
    }

    #[test]
    fn validate_known_declaration_heads_still_rejects_leading_typo_with_whitespace() {
        validate_known_declaration_heads(
            "  .inupt {$x}",
            crate::syntax::span::SourceContext::Line {
                line: 1,
                column_offset: 0,
            },
        )
        .expect_err("leading typo after whitespace must still be rejected");
    }

    #[test]
    fn parses_markup_open_tag() {
        let src = "{#bold}";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(matches!(
            exprs[0].payload,
            Some(ExpressionPayloadNode::Markup(_))
        ));
    }

    #[test]
    fn parses_markup_close_tag() {
        let src = "{/bold}";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        if let Some(ExpressionPayloadNode::Markup(m)) = &exprs[0].payload {
            assert_eq!(m.kind, crate::syntax::ast::MarkupKind::Close);
            assert_eq!(m.identifier, "bold");
        } else {
            panic!("expected markup");
        }
    }

    #[test]
    fn parses_markup_self_close_tag() {
        let src = "{#img src=|photo.jpg| /}";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        if let Some(ExpressionPayloadNode::Markup(m)) = &exprs[0].payload {
            assert_eq!(m.kind, crate::syntax::ast::MarkupKind::SelfClose);
            assert_eq!(m.identifier, "img");
            assert_eq!(m.options.len(), 1);
            assert_eq!(m.options[0].name, "src");
        } else {
            panic!("expected markup");
        }
    }

    // ─── Unquoted literal parsing ────────────────────────────────────

    #[test]
    fn unquoted_literal_bare_minus() {
        // `{-}` should parse as a literal expression with value `-`
        let doc = parse_document("{-}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        if let Some(ExpressionPayloadNode::Literal(lit)) = &exprs[0].payload {
            assert_eq!(lit.value, "-");
        } else {
            panic!("expected literal, got {:?}", exprs[0].payload);
        }
    }

    #[test]
    fn unquoted_literal_middle_dot() {
        // `{·}` (U+00B7) should parse as literal
        let doc = parse_document("{·}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(matches!(
            exprs[0].payload,
            Some(ExpressionPayloadNode::Literal(_))
        ));
    }

    #[test]
    fn unquoted_literal_invalid_number_forms() {
        // Invalid number-like tokens should still parse as unquoted literals
        for src in &[
            "{00 :number}",
            "{042 :number}",
            "{1. :number}",
            "{1e :number}",
            "{1E :number}",
            "{1.e :number}",
            "{1.2e :number}",
            "{1.e3 :number}",
            "{1e+ :number}",
            "{1e- :number}",
            "{1.0e2.0 :number}",
        ] {
            let doc = parse_document(src);
            let body = doc.body.unwrap_or_else(|| panic!("body for {src:?}"));
            let exprs = expression_segments(&body);
            assert_eq!(exprs.len(), 1, "wrong expr count for {src:?}");
            assert!(
                exprs[0].payload.is_some(),
                "payload should be Some for {src:?}, got None (syntax error)"
            );
        }
    }

    // ─── Expression strictness ───────────────────────────────────────

    #[test]
    fn expression_rejects_junk_after_literal() {
        // `{|foo| #markup}` — `#markup` after literal should produce error node
        let doc = parse_document("{|foo| #markup}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(
            exprs[0].payload.is_none(),
            "expected None payload (error node) for junk after literal"
        );
    }

    #[test]
    fn expression_rejects_missing_space_before_attribute() {
        // `{|foo|@bar}` — missing space before attribute
        let doc = parse_document("{|foo|@bar}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(
            exprs[0].payload.is_none(),
            "expected None payload for missing space before attribute"
        );
    }

    #[test]
    fn expression_rejects_junk_after_variable() {
        // `{$placeholder option}` — `option` after variable should produce error
        let doc = parse_document("{$placeholder option}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(
            exprs[0].payload.is_none(),
            "expected None payload (error node) for junk after variable"
        );
    }

    #[test]
    fn expression_allows_attribute_after_variable() {
        // `{$x @c}` — variable with attribute should work (space is present)
        let doc = parse_document("{$x @c}");
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        assert!(
            matches!(exprs[0].payload, Some(ExpressionPayloadNode::Var(_))),
            "expected Var payload for {{$x @c}}"
        );
    }

    // ─── Bidi in variable names ──────────────────────────────────────

    #[test]
    fn variable_with_leading_bidi_control() {
        // `{$\u{061C}foo}` — bidi ALM before name should be skipped
        let src = "{$\u{061C}foo}";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        if let Some(ExpressionPayloadNode::Var(v)) = &exprs[0].payload {
            assert_eq!(v.name, "foo");
        } else {
            panic!("expected Var payload, got {:?}", exprs[0].payload);
        }
    }

    #[test]
    fn variable_with_surrounding_bidi_controls() {
        // `{$\u{200E}foo\u{200F}}` — LRM before and RLM after name
        let src = "{$\u{200E}foo\u{200F}}";
        let doc = parse_document(src);
        let body = doc.body.expect("body");
        let exprs = expression_segments(&body);
        assert_eq!(exprs.len(), 1);
        if let Some(ExpressionPayloadNode::Var(v)) = &exprs[0].payload {
            assert_eq!(v.name, "foo");
        } else {
            panic!("expected Var payload, got {:?}", exprs[0].payload);
        }
    }

    #[test]
    fn local_decl_with_bidi_in_variable_name() {
        // `.local $\u{200E}foo\u{200F} = {3} {{{$foo}}}`
        let src = ".local $\u{200E}foo\u{200F} = {3} {{{$foo}}}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 1);
        if let Some(DeclarationPayloadNode::Local { name, .. }) = &doc.declarations[0].payload {
            assert_eq!(*name, "foo");
        } else {
            panic!(
                "expected Local payload, got {:?}",
                doc.declarations[0].payload
            );
        }
    }

    // ─── Match syntax strictness ─────────────────────────────────────

    #[test]
    fn match_rejects_no_whitespace_after_selector() {
        // `.match $x* {{foo}}` — no space between selector and `*`
        let src = ".input {$x :x} .match $x* {{foo}}";
        let doc = parse_document(src);
        // The match should fail to parse (no payload) since there's no
        // whitespace between $x and the variant key
        let match_decl = doc
            .declarations
            .iter()
            .find(|d| d.kind == DeclarationKind::Match)
            .expect("match decl");
        assert!(
            match_decl.payload.is_none(),
            "expected None payload for `.match $x*` (no space after selector)"
        );
    }

    #[test]
    fn match_rejects_double_star_keys() {
        // `.match $x $y ** {{foo}}` — `**` without space is a single key, not two
        let src = ".input {$x :x} .local $y = {y :y} .match $x $y ** {{foo}}";
        let doc = parse_document(src);
        let match_decl = doc
            .declarations
            .iter()
            .find(|d| d.kind == DeclarationKind::Match)
            .expect("match decl");
        // With mandatory whitespace between keys, `**` is one key (not `*` `*`),
        // so the variant has 1 key for 2 selectors → parse failure
        if let Some(DeclarationPayloadNode::Match { variants, .. }) = &match_decl.payload {
            // If parsed, the single `**` key should mismatch 2 selectors
            assert_eq!(variants[0].keys.len(), 1);
        }
    }

    // ─── Standalone quoted-pattern ────────────────────────────────────

    #[test]
    fn parses_standalone_quoted_pattern_body() {
        let src = "{{Hello}}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 0);
        let body = doc.body.expect("body");
        // Body span covers the entire source (frontend extracts inner content)
        assert_eq!(&src[body.span.clone()], "{{Hello}}");
    }

    #[test]
    fn parses_declarations_with_quoted_pattern_body() {
        let src = ".input {$x} {{Hello {$x}}}";
        let doc = parse_document(src);
        assert_eq!(doc.declarations.len(), 1);
        let body = doc.body.expect("body");
        // Body span includes the `{{...}}` wrapper
        assert!(src[body.span.clone()].contains("Hello"));
    }
}
