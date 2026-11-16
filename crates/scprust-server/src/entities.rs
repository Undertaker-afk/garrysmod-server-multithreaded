//! Bounded quoted-key/value entity-lump subset. Duplicate properties are kept
//! (important for repeated entity I/O outputs); values are bytes, not forced UTF-8.
use crate::bsp::{Error, Result};
pub const MAX_ENTITY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTITIES: usize = 16_384;
const MAX_PAIRS: usize = 131_072;
#[derive(Debug)]
pub struct Entity {
    pub properties: Vec<(Vec<u8>, Vec<u8>)>,
}
impl Entity {
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.properties
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_slice())
    }
}
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Lexer<'_> {
    fn whitespace(&mut self) {
        loop {
            while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
            if self.bytes.get(self.at..self.at + 2) == Some(b"//") {
                while self.at < self.bytes.len() && self.bytes[self.at] != b'\n' {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }
    fn quoted(&mut self) -> Result<Vec<u8>> {
        self.whitespace();
        if self.bytes.get(self.at) != Some(&b'"') {
            return Err(Error("expected quoted entity token"));
        }
        self.at += 1;
        let mut out = Vec::new();
        while let Some(&byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                0 => return Err(Error("NUL inside entity token")),
                b'\\' if matches!(self.bytes.get(self.at), Some(b'"' | b'\\')) => {
                    out.push(self.bytes[self.at]);
                    self.at += 1;
                }
                _ => out.push(byte),
            }
            if out.len() > 4096 {
                return Err(Error("entity token too long"));
            }
        }
        Err(Error("unterminated entity token"))
    }
}
pub fn parse(bytes: &[u8]) -> Result<Vec<Entity>> {
    if bytes.len() > MAX_ENTITY_BYTES {
        return Err(Error("entity lump too large"));
    }
    let mut lexer = Lexer { bytes, at: 0 };
    let mut entities = Vec::new();
    let mut pairs = 0;
    loop {
        lexer.whitespace();
        match lexer.bytes.get(lexer.at) {
            None => return Ok(entities),
            Some(0) => {
                if lexer.bytes[lexer.at..]
                    .iter()
                    .all(|b| *b == 0 || b.is_ascii_whitespace())
                {
                    return Ok(entities);
                }
                return Err(Error("data follows entity terminator"));
            }
            Some(b'{') => lexer.at += 1,
            _ => return Err(Error("expected entity opening brace")),
        }
        if entities.len() >= MAX_ENTITIES {
            return Err(Error("too many entities"));
        }
        let mut properties = Vec::new();
        loop {
            lexer.whitespace();
            if lexer.bytes.get(lexer.at) == Some(&b'}') {
                lexer.at += 1;
                break;
            }
            if properties.len() >= 256 || pairs >= MAX_PAIRS {
                return Err(Error("too many entity properties"));
            }
            let key = lexer.quoted()?;
            let value = lexer.quoted()?;
            properties.push((key, value));
            pairs += 1;
        }
        entities.push(Entity { properties });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_duplicate_keys_paths_escapes_and_non_utf8() {
        let entities = parse(b"// comment\n{\"classname\" \"worldspawn\" \"output\" \"a\" \"output\" \"b\" \"path\" \"a\\materials\" \"quote\" \"a\\\"b\" \"raw\" \"\xff\"}\0").unwrap();
        assert_eq!(entities[0].get(b"output"), Some(&b"b"[..]));
        assert_eq!(entities[0].get(b"path"), Some(&b"a\\materials"[..]));
        assert_eq!(entities[0].get(b"quote"), Some(&b"a\"b"[..]));
        assert_eq!(entities[0].get(b"raw"), Some(&b"\xff"[..]));
        assert_eq!(entities[0].properties.len(), 6);
    }
    #[test]
    fn rejects_malformed_and_unbounded_input() {
        for bytes in [
            &b"{"[..],
            b"{ a b }",
            b"{\"a\"}",
            b"{\"a\"\"b",
            b"{}\0garbage",
            b"{\"a\"\"\0\"}",
        ] {
            assert!(parse(bytes).is_err());
        }
        let long = format!("{{\"a\"\"{}\"}}", "x".repeat(4097));
        assert!(parse(long.as_bytes()).is_err());
        let pairs = format!("{{{}}}", "\"a\"\"b\"".repeat(257));
        assert!(parse(pairs.as_bytes()).is_err());
    }
}
