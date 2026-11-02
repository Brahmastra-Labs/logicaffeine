//! A small, allocation-free JSON reader for one NDJSON record at a time.
//!
//! A record is parsed into a flat token tape that is reused across lines; containers record where
//! they end so a lookup can skip a whole value in O(1). Strings stay borrowed from the line and are
//! decoded only when a name or literal actually needs their text.

#[derive(Clone, Copy, Debug)]
pub enum Tok<'a> {
    Obj { end: u32 },
    Arr { end: u32 },
    Key(&'a [u8], bool),
    Str(&'a [u8], bool),
    Num(&'a [u8]),
    True,
    False,
    Null,
}

pub struct Tape<'a> {
    pub toks: Vec<Tok<'a>>,
}

#[derive(Debug)]
pub struct JsonError;

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    #[inline]
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn string(&mut self) -> Result<(&'a [u8], bool), JsonError> {
        // at opening quote
        self.i += 1;
        let start = self.i;
        let mut escaped = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    let s = &self.b[start..self.i];
                    self.i += 1;
                    return Ok((s, escaped));
                }
                b'\\' => {
                    escaped = true;
                    self.i += 2;
                }
                c if c < 0x20 => return Err(JsonError),
                _ => self.i += 1,
            }
        }
        Err(JsonError)
    }

    fn value(&mut self, toks: &mut Vec<Tok<'a>>, depth: u32) -> Result<(), JsonError> {
        if depth > 64 {
            return Err(JsonError);
        }
        self.ws();
        let c = *self.b.get(self.i).ok_or(JsonError)?;
        match c {
            b'{' => {
                let at = toks.len();
                toks.push(Tok::Obj { end: 0 });
                self.i += 1;
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        if self.b.get(self.i) != Some(&b'"') {
                            return Err(JsonError);
                        }
                        let (k, e) = self.string()?;
                        toks.push(Tok::Key(k, e));
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return Err(JsonError);
                        }
                        self.i += 1;
                        self.value(toks, depth + 1)?;
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(JsonError),
                        }
                    }
                }
                toks[at] = Tok::Obj {
                    end: toks.len() as u32,
                };
            }
            b'[' => {
                let at = toks.len();
                toks.push(Tok::Arr { end: 0 });
                self.i += 1;
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        self.value(toks, depth + 1)?;
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(JsonError),
                        }
                    }
                }
                toks[at] = Tok::Arr {
                    end: toks.len() as u32,
                };
            }
            b'"' => {
                let (s, e) = self.string()?;
                toks.push(Tok::Str(s, e));
            }
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                toks.push(Tok::True);
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                toks.push(Tok::False);
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                toks.push(Tok::Null);
            }
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                    )
                {
                    self.i += 1;
                }
                toks.push(Tok::Num(&self.b[start..self.i]));
            }
            _ => return Err(JsonError),
        }
        Ok(())
    }
}

impl<'a> Tape<'a> {
    /// Parses one JSON document (the whole of `line`, trailing whitespace allowed).
    pub fn parse(
        line: &'a [u8],
        mut toks: Vec<Tok<'a>>,
    ) -> Result<Tape<'a>, (JsonError, Vec<Tok<'a>>)> {
        toks.clear();
        let mut p = P { b: line, i: 0 };
        match p.value(&mut toks, 0) {
            Ok(()) => {
                p.ws();
                if p.i != line.len() {
                    return Err((JsonError, toks));
                }
                Ok(Tape { toks })
            }
            Err(e) => Err((e, toks)),
        }
    }

    #[inline]
    pub fn skip(&self, i: usize) -> usize {
        match self.toks[i] {
            Tok::Obj { end } | Tok::Arr { end } => end as usize,
            _ => i + 1,
        }
    }

    /// The members of the object at `i`, as (key, value index).
    pub fn members(&self, i: usize) -> impl Iterator<Item = (&'a [u8], usize)> + '_ {
        let end = match self.toks[i] {
            Tok::Obj { end } => end as usize,
            _ => i + 1,
        };
        let mut j = i + 1;
        std::iter::from_fn(move || {
            if j >= end {
                return None;
            }
            let Tok::Key(k, _) = self.toks[j] else {
                return None;
            };
            let v = j + 1;
            j = self.skip(v);
            Some((k, v))
        })
    }

    pub fn get(&self, obj: usize, key: &str) -> Option<usize> {
        self.members(obj)
            .find(|(k, _)| *k == key.as_bytes())
            .map(|(_, v)| v)
    }

    pub fn elems(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let end = match self.toks[i] {
            Tok::Arr { end } => end as usize,
            _ => i + 1,
        };
        let mut j = i + 1;
        std::iter::from_fn(move || {
            if j >= end {
                return None;
            }
            let v = j;
            j = self.skip(v);
            Some(v)
        })
    }

    /// A non-negative integer that fits `u64`; anything else (sign, fraction, exponent,
    /// overflow) is `None`.
    #[inline]
    pub fn u64(&self, i: usize) -> Option<u64> {
        let Tok::Num(s) = self.toks[i] else {
            return None;
        };
        if s.is_empty() || s.len() > 20 {
            return None;
        }
        let mut v: u64 = 0;
        for &b in s {
            let d = b.wrapping_sub(b'0');
            if d > 9 {
                return None;
            }
            v = v.checked_mul(10)?.checked_add(d as u64)?;
        }
        Some(v)
    }

    pub fn bool(&self, i: usize) -> Option<bool> {
        match self.toks[i] {
            Tok::True => Some(true),
            Tok::False => Some(false),
            _ => None,
        }
    }

    pub fn str(&self, i: usize) -> Option<std::borrow::Cow<'a, str>> {
        match self.toks[i] {
            Tok::Str(s, false) => std::str::from_utf8(s).ok().map(std::borrow::Cow::Borrowed),
            Tok::Str(s, true) => unescape(s).map(std::borrow::Cow::Owned),
            _ => None,
        }
    }
}

fn hex4(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(&b[..4]).ok()?, 16).ok()
}

fn unescape(s: &[u8]) -> Option<String> {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let c = *s.get(i + 1)?;
        i += 2;
        let ch = match c {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let hi = hex4(&s[i..])?;
                i += 4;
                if (0xD800..0xDC00).contains(&hi) {
                    if s.get(i) != Some(&b'\\') || s.get(i + 1) != Some(&b'u') {
                        return None;
                    }
                    let lo = hex4(&s[i + 2..])?;
                    i += 6;
                    if !(0xDC00..0xE000).contains(&lo) {
                        return None;
                    }
                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))?
                } else {
                    char::from_u32(hi)?
                }
            }
            _ => return None,
        };
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/json/tests.rs"]
mod tests;
