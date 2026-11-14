//! Generic parser for the nested `{ name(type)=value, ... }` / `[count, items...]` syntax.

use crate::MapError;

#[derive(Debug, Clone)]
pub struct Value {
    pub line: u32,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Num(f64),
    Bool(bool),
    /// Bare word such as an enum name (`fi_player`, `fs_intact`).
    Ident(String),
    Object(Vec<Field>),
    /// `[count, item, ...]`; `count` is the declared count, `items` what was actually present.
    Array { count: usize, items: Vec<Value> },
    /// `tag={...}` in value position (e.g. `sun={...}`).
    Tagged(String, Box<Value>),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    #[allow(dead_code)] // informational per MAPFORMAT.md

    pub ty: String,
    pub line: u32,
    pub value: Value,
}

const MAX_DEPTH: usize = 64;

pub struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    line_start: usize,
}

impl<'a> Parser<'a> {
    pub fn new(text: &'a str) -> Self {
        Parser { src: text.as_bytes(), pos: 0, line: 1, line_start: 0 }
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, MapError> {
        Err(MapError::at(self.line, (self.pos - self.line_start + 1) as u32, msg))
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            match c {
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                }
                b' ' | b'\t' | b'\r' => self.pos += 1,
                // tolerate a UTF-8 BOM at the very start
                0xEF if self.pos == 0 && self.src.starts_with(&[0xEF, 0xBB, 0xBF]) => self.pos += 3,
                _ => break,
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), MapError> {
        self.skip_ws();
        match self.peek() {
            Some(x) if x == c => {
                self.pos += 1;
                Ok(())
            }
            Some(x) => self.err(format!("expected '{}' but found '{}'", c as char, x as char)),
            None => self.err(format!("expected '{}' but reached end of file", c as char)),
        }
    }

    fn ident(&mut self) -> Result<String, MapError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return match self.peek() {
                Some(c) => self.err(format!("expected a name but found '{}'", c as char)),
                None => self.err("expected a name but reached end of file"),
            };
        }
        Ok(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    /// Parse a complete document: exactly one value followed only by whitespace.
    pub fn parse_document(mut self) -> Result<Value, MapError> {
        let v = self.value(0)?;
        self.skip_ws();
        if self.pos < self.src.len() {
            return self.err("unexpected trailing content after the top-level value");
        }
        Ok(v)
    }

    fn value(&mut self, depth: usize) -> Result<Value, MapError> {
        if depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        self.skip_ws();
        let line = self.line;
        let c = match self.peek() {
            Some(c) => c,
            None => return self.err("expected a value but reached end of file"),
        };
        let kind = match c {
            b'{' => self.object(depth)?,
            b'[' => self.array(depth)?,
            b'0'..=b'9' | b'-' | b'+' | b'.' => Kind::Num(self.number()?),
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let id = self.ident()?;
                match id.as_str() {
                    "true" => Kind::Bool(true),
                    "false" => Kind::Bool(false),
                    _ => {
                        self.skip_ws();
                        if self.peek() == Some(b'=') {
                            self.pos += 1;
                            let inner = self.value(depth + 1)?;
                            Kind::Tagged(id, Box::new(inner))
                        } else {
                            Kind::Ident(id)
                        }
                    }
                }
            }
            c => return self.err(format!("unexpected character '{}'", c as char)),
        };
        Ok(Value { line, kind })
    }

    fn number(&mut self) -> Result<f64, MapError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        match s.parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(v),
            _ => {
                self.pos = start;
                self.err(format!("malformed number '{s}'"))
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Kind, MapError> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b',') => {
                    // tolerate stray / trailing commas
                    self.pos += 1;
                    continue;
                }
                None => return self.err("unterminated object: missing '}'"),
                _ => {}
            }
            let line = self.line;
            let name = self.ident()?;
            self.skip_ws();
            let mut ty = String::new();
            if self.peek() == Some(b'(') {
                self.pos += 1;
                let s = self.pos;
                while let Some(c) = self.peek() {
                    if c == b')' {
                        break;
                    }
                    if c == b'\n' {
                        return self.err(format!("unterminated type annotation on field '{name}'"));
                    }
                    self.pos += 1;
                }
                if self.peek() != Some(b')') {
                    return self.err(format!("unterminated type annotation on field '{name}'"));
                }
                ty = String::from_utf8_lossy(&self.src[s..self.pos]).into_owned();
                self.pos += 1;
            }
            self.skip_ws();
            if self.peek() != Some(b'=') {
                return self.err(format!("expected '=' after field name '{name}'"));
            }
            self.pos += 1;
            let value = self.value(depth + 1)?;
            fields.push(Field { name, ty, line, value });
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {}
                Some(c) => return self.err(format!("expected ',' or '}}' after field, found '{}'", c as char)),
                None => return self.err("unterminated object: missing '}'"),
            }
        }
        Ok(Kind::Object(fields))
    }

    fn array(&mut self, depth: usize) -> Result<Kind, MapError> {
        self.expect(b'[')?;
        self.skip_ws();
        let count_line = self.line;
        let cv = self.value(depth + 1)?;
        let count = match cv.kind {
            Kind::Num(n) if n >= 0.0 && n.fract() == 0.0 && n < 1e7 => n as usize,
            _ => return Err(MapError::at(count_line, 0, "array must start with a non-negative integer item count")),
        };
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        continue;
                    }
                    items.push(self.value(depth + 1)?);
                }
                Some(c) => return self.err(format!("expected ',' or ']' in array, found '{}'", c as char)),
                None => return self.err("unterminated array: missing ']'"),
            }
        }
        Ok(Kind::Array { count, items })
    }
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self.kind {
            Kind::Num(n) => Some(n),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&[Field]> {
        match &self.kind {
            Kind::Object(f) => Some(f),
            Kind::Tagged(_, inner) => inner.as_object(),
            _ => None,
        }
    }
}
