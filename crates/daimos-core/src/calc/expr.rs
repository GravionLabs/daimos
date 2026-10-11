//! Arithmetic: a tokenizer, a recursive-descent parser and an evaluator.
//!
//! Precedence, loosest to tightest: `+ -`, `* / mod` (and `x% of y`), unary `-`, `^` (right
//! associative), postfix `%`, then numbers, constants, functions and parentheses. `-2^2` is `-4`,
//! as in mathematics.

#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// A number; `special` is set for hex, binary, octal and scientific notation, which are worth
    /// showing even though they are "just a number".
    Num(f64, bool),
    Ident(String),
    Op(char),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Num(f64, bool),
    Const(f64),
    Neg(Box<Ast>),
    Bin(char, Box<Ast>, Box<Ast>),
    Pct(Box<Ast>),
    PctOf(Box<Ast>, Box<Ast>),
    Call(Func, Box<Ast>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Func {
    Sqrt,
    Cbrt,
    Abs,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log,
    Log2,
    Exp,
    Floor,
    Ceil,
    Round,
}

fn function(name: &str) -> Option<Func> {
    Some(match name {
        "sqrt" => Func::Sqrt,
        "cbrt" => Func::Cbrt,
        "abs" => Func::Abs,
        "sin" => Func::Sin,
        "cos" => Func::Cos,
        "tan" => Func::Tan,
        "asin" => Func::Asin,
        "acos" => Func::Acos,
        "atan" => Func::Atan,
        "ln" => Func::Ln,
        "log" => Func::Log,
        "log2" => Func::Log2,
        "exp" => Func::Exp,
        "floor" => Func::Floor,
        "ceil" => Func::Ceil,
        "round" => Func::Round,
        _ => return None,
    })
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" | "π" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "tau" => Some(std::f64::consts::TAU),
        _ => None,
    }
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let (token, next) = number(&chars, i)?;
            tokens.push(token);
            i = next;
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect::<String>().to_lowercase()));
        } else {
            let op = match c {
                '+' | '-' | '^' | '%' | '(' | ')' => c,
                '*' | '×' | '·' => '*',
                '/' | '÷' => '/',
                '−' => '-',
                _ => return None,
            };
            tokens.push(Token::Op(op));
            i += 1;
        }
    }
    Some(tokens)
}

/// A number starting at `start`: `12`, `1.5`, `.5`, `1e6`, `2.5e-3`, `0xff`, `0b101`, `0o17`.
fn number(chars: &[char], start: usize) -> Option<(Token, usize)> {
    let mut i = start;
    if chars[i] == '0'
        && let Some(radix) = chars.get(i + 1).and_then(|c| match c.to_ascii_lowercase() {
            'x' => Some(16),
            'b' => Some(2),
            'o' => Some(8),
            _ => None,
        })
    {
        let digits_start = i + 2;
        let mut end = digits_start;
        while end < chars.len() && chars[end].is_digit(radix) {
            end += 1;
        }
        if end > digits_start {
            let text: String = chars[digits_start..end].iter().collect();
            return Some((Token::Num(u64::from_str_radix(&text, radix).ok()? as f64, true), end));
        }
    }
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
        i += 1;
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    let mut special = false;
    // An exponent only counts when digits follow: `2e` is 2 times the constant e.
    if matches!(chars.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+' | '-')) {
            j += 1;
        }
        if chars.get(j).is_some_and(char::is_ascii_digit) {
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
            special = true;
        }
    }
    let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
    Some((Token::Num(text.parse().ok()?, special), i))
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn op(&mut self, wanted: char) -> bool {
        if self.peek() == Some(&Token::Op(wanted)) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self, wanted: &str) -> bool {
        if matches!(self.peek(), Some(Token::Ident(name)) if name == wanted) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<Ast> {
        let mut left = self.term()?;
        loop {
            let op = if self.op('+') {
                '+'
            } else if self.op('-') {
                '-'
            } else {
                return Some(left);
            };
            left = Ast::Bin(op, Box::new(left), Box::new(self.term()?));
        }
    }

    fn term(&mut self) -> Option<Ast> {
        let mut left = self.unary()?;
        loop {
            if self.op('*') {
                left = Ast::Bin('*', Box::new(left), Box::new(self.unary()?));
            } else if self.op('/') {
                left = Ast::Bin('/', Box::new(left), Box::new(self.unary()?));
            } else if self.ident("mod") {
                left = Ast::Bin('m', Box::new(left), Box::new(self.unary()?));
            } else if let Ast::Pct(percent) = &left
                && self.ident("of")
            {
                // `20% of 50% of 80`: the base may itself be a percentage of something.
                let mut base = self.unary()?;
                while let Ast::Pct(inner) = &base
                    && self.ident("of")
                {
                    let inner = inner.clone();
                    base = Ast::PctOf(inner, Box::new(self.unary()?));
                }
                left = Ast::PctOf(percent.clone(), Box::new(base));
            } else {
                return Some(left);
            }
        }
    }

    fn unary(&mut self) -> Option<Ast> {
        if self.op('-') {
            Some(Ast::Neg(Box::new(self.unary()?)))
        } else if self.op('+') {
            self.unary()
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> Option<Ast> {
        let base = self.postfix()?;
        if self.op('^') {
            Some(Ast::Bin('^', Box::new(base), Box::new(self.unary()?)))
        } else {
            Some(base)
        }
    }

    fn postfix(&mut self) -> Option<Ast> {
        let mut value = self.primary()?;
        while self.op('%') {
            value = Ast::Pct(Box::new(value));
        }
        Some(value)
    }

    fn primary(&mut self) -> Option<Ast> {
        match self.next()? {
            Token::Num(value, special) => Some(Ast::Num(value, special)),
            Token::Op('(') => {
                let inner = self.expr()?;
                self.op(')').then_some(inner)
            }
            Token::Ident(name) => {
                if let Some(func) = function(&name) {
                    // `sqrt 2`, `sqrt(2)`, `sqrt 2 + 1` is `sqrt(2) + 1`.
                    Some(Ast::Call(func, Box::new(self.unary()?)))
                } else {
                    constant(&name).map(Ast::Const)
                }
            }
            Token::Op(_) => None,
        }
    }
}

/// Parses a whole input; anything left over means it is not an expression.
pub fn parse(input: &str) -> Option<Ast> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return None;
    }
    let mut parser = Parser { tokens, position: 0 };
    let ast = parser.expr()?;
    (parser.position == parser.tokens.len()).then_some(ast)
}

/// A number or a constant alone is not worth an item (typing `e` or `42` is a search).
pub fn is_trivial(ast: &Ast) -> bool {
    matches!(ast, Ast::Num(_, false) | Ast::Const(_))
}

/// The value, or `None` for division by zero, a domain error or an overflow.
pub fn eval(ast: &Ast) -> Option<f64> {
    let value = match ast {
        Ast::Num(value, _) | Ast::Const(value) => *value,
        Ast::Neg(inner) => -eval(inner)?,
        Ast::Pct(inner) => eval(inner)? / 100.0,
        Ast::PctOf(percent, base) => eval(percent)? / 100.0 * eval(base)?,
        Ast::Bin(op, left, right) => {
            let l = eval(left)?;
            // `80 + 10%` is 80 plus 10 % of 80, not 80.1.
            if let ('+' | '-', Ast::Pct(percent)) = (op, right.as_ref()) {
                let amount = l * eval(percent)? / 100.0;
                return finite(if *op == '+' { l + amount } else { l - amount });
            }
            let r = eval(right)?;
            match op {
                '+' => l + r,
                '-' => l - r,
                '*' => l * r,
                '/' => l / r,
                'm' => l % r,
                '^' => l.powf(r),
                _ => return None,
            }
        }
        Ast::Call(func, arg) => {
            let x = eval(arg)?;
            match func {
                Func::Sqrt => x.sqrt(),
                Func::Cbrt => x.cbrt(),
                Func::Abs => x.abs(),
                Func::Sin => x.sin(),
                Func::Cos => x.cos(),
                Func::Tan => x.tan(),
                Func::Asin => x.asin(),
                Func::Acos => x.acos(),
                Func::Atan => x.atan(),
                Func::Ln => x.ln(),
                Func::Log => x.log10(),
                Func::Log2 => x.log2(),
                Func::Exp => x.exp(),
                Func::Floor => x.floor(),
                Func::Ceil => x.ceil(),
                Func::Round => x.round(),
            }
        }
    };
    finite(value)
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(input: &str) -> Option<f64> {
        parse(input).and_then(|ast| eval(&ast))
    }

    fn close(input: &str, expected: f64) {
        let got = calc(input).unwrap_or_else(|| panic!("{input} did not evaluate"));
        assert!(
            (got - expected).abs() < 1e-9 * expected.abs().max(1.0),
            "{input} = {got}, expected {expected}"
        );
    }

    #[test]
    fn the_examples_of_the_acceptance_criteria() {
        close("2*(3+4)", 14.0);
        close("sqrt 2", 2f64.sqrt());
        close("2^10", 1024.0);
        close("10% of 80", 8.0);
        close("0xff", 255.0);
        close("1e6 / 7", 1e6 / 7.0);
    }

    #[test]
    fn precedence_and_associativity() {
        close("2+3*4", 14.0);
        close("(2+3)*4", 20.0);
        close("2^3^2", 512.0); // right associative
        close("-2^2", -4.0);
        close("2*-3", -6.0);
        close("10-4-3", 3.0);
        close("100/10/5", 2.0);
        close("7 mod 4", 3.0);
        close("2 * 3 mod 4", 2.0);
    }

    #[test]
    fn functions_constants_and_other_radixes() {
        close("sqrt(16) + 1", 5.0);
        close("sqrt 16 + 1", 5.0);
        close("sin(pi/2)", 1.0);
        close("2*pi", 2.0 * std::f64::consts::PI);
        close("ln e", 1.0);
        close("log 1000", 3.0);
        close("0b101 + 0o17", 20.0);
        close("1_000 + 1", 1001.0);
        close("1.5e-3 * 2", 0.003);
        // No implicit multiplication: `2e` is not an expression.
        assert_eq!(calc("2e"), None);
    }

    #[test]
    fn percentages() {
        close("50%", 0.5);
        close("80 + 10%", 88.0);
        close("80 - 25%", 60.0);
        close("200 * 10%", 20.0);
        close("20% of 50% of 80", 8.0);
    }

    #[test]
    fn what_is_not_an_expression_gives_nothing() {
        for input in [
            "", "   ", "firefox", "calc 2", "2 +", "(2", "2)", "* 3", "1/0", "sqrt -1", "ln 0",
            "2 $ 3", "pi pi",
        ] {
            assert_eq!(calc(input), None, "{input:?}");
        }
    }

    #[test]
    fn a_bare_number_or_constant_is_not_worth_an_item() {
        for input in ["42", "3.14", "e", "pi"] {
            assert!(parse(input).is_some_and(|a| is_trivial(&a)), "{input}");
        }
        for input in ["0xff", "1e6", "2+2", "-3", "sqrt 4", "5%"] {
            assert!(parse(input).is_some_and(|a| !is_trivial(&a)), "{input}");
        }
    }

    #[test]
    fn unicode_operators_work() {
        close("6 × 7", 42.0);
        close("84 ÷ 2", 42.0);
        close("5 − 3", 2.0);
    }
}
