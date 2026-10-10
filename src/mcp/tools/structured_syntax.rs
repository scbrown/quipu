//! Bounded boolean syntax for explicit structured search requests.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    Atom(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Atom(String),
    Left,
    Right,
    And,
    Or,
    Not,
}
#[derive(Debug, PartialEq)]
pub(super) struct ParseError {
    pub offset: usize,
    pub message: &'static str,
}
type Result<T> = std::result::Result<T, ParseError>;
fn lex(input: &str) -> Result<Vec<(usize, Token)>> {
    if input.len() > 4096 {
        return Err(ParseError {
            offset: 4096,
            message: "expression exceeds 4096 bytes",
        });
    }
    let mut out = Vec::new();
    let mut it = input.char_indices().peekable();
    while let Some((offset, c)) = it.next() {
        if c.is_whitespace() {
            continue;
        }
        let token = match c {
            '(' => Token::Left,
            ')' => Token::Right,
            '-' => Token::Not,
            _ => {
                let mut atom = String::from(c);
                let mut quote = c == '"';
                let mut escaped = false;
                while let Some(&(_, next)) = it.peek() {
                    if !quote && (next.is_whitespace() || next == '(' || next == ')') {
                        break;
                    }
                    it.next();
                    atom.push(next);
                    if escaped {
                        escaped = false;
                    } else if next == '\\' && quote {
                        escaped = true;
                    } else if next == '"' {
                        quote = !quote;
                    }
                }
                if quote || escaped {
                    return Err(ParseError {
                        offset,
                        message: "unterminated quoted phrase",
                    });
                }
                match atom.as_str() {
                    "AND" => Token::And,
                    "OR" => Token::Or,
                    "NOT" => Token::Not,
                    _ => Token::Atom(atom),
                }
            }
        };
        out.push((offset, token));
        if out.len() > 64 {
            return Err(ParseError {
                offset,
                message: "expression exceeds 64 tokens",
            });
        }
    }
    Ok(out)
}
struct Parser {
    tokens: Vec<(usize, Token)>,
    cursor: usize,
    end: usize,
}
impl Parser {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            offset: self.tokens.get(self.cursor).map_or(self.end, |t| t.0),
            message,
        }
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor).map(|t| &t.1)
    }
    fn take(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self, depth: usize) -> Result<Expr> {
        let mut expr = self.and(depth)?;
        while self.take(&Token::Or) {
            expr = Expr::Or(Box::new(expr), Box::new(self.and(depth)?));
        }
        Ok(expr)
    }
    fn and(&mut self, depth: usize) -> Result<Expr> {
        let mut expr = self.unary(depth)?;
        loop {
            let explicit = self.take(&Token::And);
            if !explicit && !matches!(self.peek(), Some(Token::Atom(_) | Token::Not | Token::Left))
            {
                break;
            }
            expr = Expr::And(Box::new(expr), Box::new(self.unary(depth)?));
        }
        Ok(expr)
    }
    fn unary(&mut self, depth: usize) -> Result<Expr> {
        if depth > 8 {
            return Err(self.error("expression exceeds nesting depth 8"));
        }
        if self.take(&Token::Not) {
            return Ok(Expr::Not(Box::new(self.unary(depth + 1)?)));
        }
        if self.take(&Token::Left) {
            let expr = self.or(depth + 1)?;
            if !self.take(&Token::Right) {
                return Err(self.error("expected closing parenthesis"));
            }
            return Ok(expr);
        }
        match self.peek().cloned() {
            Some(Token::Atom(atom)) => {
                self.cursor += 1;
                Ok(Expr::Atom(atom))
            }
            _ => Err(self.error("expected term or field comparison")),
        }
    }
}
pub(super) fn parse(input: &str) -> Result<Expr> {
    let mut parser = Parser {
        tokens: lex(input)?,
        cursor: 0,
        end: input.len(),
    };
    let expr = parser.or(0)?;
    if parser.peek().is_some() {
        return Err(parser.error("unexpected token"));
    }
    Ok(expr)
}

/// A positive atom provides a finite candidate universe. NOT cannot enumerate
/// the graph complement; each OR arm needs its own positive seed.
pub(super) fn bounded(expr: &Expr, seeded: bool) -> bool {
    match expr {
        Expr::Atom(_) => true,
        Expr::Not(_) => seeded,
        Expr::And(left, right) => {
            let positive = positive(left) || positive(right);
            bounded(left, seeded || positive) && bounded(right, seeded || positive)
        }
        Expr::Or(left, right) => bounded(left, seeded) && bounded(right, seeded),
    }
}
fn positive(expr: &Expr) -> bool {
    match expr {
        Expr::Atom(_) => true,
        Expr::Not(_) => false,
        Expr::And(left, right) => positive(left) || positive(right),
        Expr::Or(left, right) => positive(left) && positive(right),
    }
}

#[cfg(test)]
#[path = "structured_syntax_tests.rs"]
mod tests;
