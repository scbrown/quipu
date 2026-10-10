//! A deliberately small, strict `COSE_Key` reader (RFC 9052 §7, RFC 9053).
//!
//! A `WebAuthn` credential public key is a CBOR map. We need five labels from it,
//! so this reads exactly the CBOR subset a `COSE_Key` uses and refuses the rest,
//! rather than pulling a general CBOR crate into a verification path:
//!
//! * definite lengths only (indefinite-length items are refused)
//! * integer map labels only, no duplicates
//! * values: unsigned/negative integers, byte strings, text strings
//!   (nested arrays/maps, tags, floats and simple values are refused)
//! * no trailing bytes after the map
//!
//! Refusing an unusual-but-valid encoding is the safe failure here: the key
//! is registered by a human once, and a refusal is loud and fixable, whereas a
//! lenient parser that reads two different keys out of one blob is not.

/// A credential public key usable for a `WebAuthn` assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoseKey {
    /// EC2 / P-256 / ES256: the uncompressed SEC1 point `04 || x || y`.
    Es256 {
        /// `04 || x || y`, 65 bytes.
        sec1_uncompressed: Vec<u8>,
    },
    /// OKP / Ed25519 / `EdDSA`: the 32-byte public key.
    Ed25519 {
        /// The 32-byte key.
        public: [u8; 32],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Int(i128),
    Bytes(Vec<u8>),
    Text,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or("COSE key: truncated CBOR")?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Read an initial byte's major type and its argument.
    fn head(&mut self) -> Result<(u8, u64), String> {
        let ib = self.take(1)?[0];
        let major = ib >> 5;
        let info = ib & 0x1f;
        let arg = match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?.try_into().unwrap_or([0; 2]),
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?.try_into().unwrap_or([0; 4]),
            )),
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap_or([0; 8])),
            _ => return Err("COSE key: indefinite or reserved CBOR length refused".into()),
        };
        Ok((major, arg))
    }

    fn len(arg: u64) -> Result<usize, String> {
        usize::try_from(arg).map_err(|_| "COSE key: CBOR length overflow".to_string())
    }

    fn item(&mut self) -> Result<Item, String> {
        let (major, arg) = self.head()?;
        match major {
            0 => Ok(Item::Int(i128::from(arg))),
            1 => Ok(Item::Int(-1 - i128::from(arg))),
            2 => Ok(Item::Bytes(self.take(Self::len(arg)?)?.to_vec())),
            3 => {
                let raw = self.take(Self::len(arg)?)?;
                std::str::from_utf8(raw).map_err(|_| "COSE key: invalid UTF-8 text")?;
                Ok(Item::Text)
            }
            _ => Err(format!("COSE key: CBOR major type {major} not allowed")),
        }
    }
}

// COSE labels and values (RFC 9052 / 9053, IANA COSE registries).
const KTY: i128 = 1;
const ALG: i128 = 3;
const CRV: i128 = -1;
const X: i128 = -2;
const Y: i128 = -3;
const KTY_OKP: i128 = 1;
const KTY_EC2: i128 = 2;
const ALG_ES256: i128 = -7;
const ALG_EDDSA: i128 = -8;
const CRV_P256: i128 = 1;
const CRV_ED25519: i128 = 6;

/// Parse a `COSE_Key`. The `alg` label is REQUIRED (an alg-less key could be
/// used under any algorithm) and must be consistent with `kty` and `crv`.
///
/// # Errors
/// On any malformed, unsupported or inconsistent key.
pub fn parse(bytes: &[u8]) -> Result<CoseKey, String> {
    let mut r = Reader { buf: bytes, pos: 0 };
    let (major, n) = r.head()?;
    if major != 5 {
        return Err("COSE key: top-level CBOR item must be a map".into());
    }
    let n = Reader::len(n)?;
    if n > 16 {
        return Err("COSE key: too many entries".into());
    }
    let mut entries: Vec<(i128, Item)> = Vec::with_capacity(n);
    for _ in 0..n {
        let Item::Int(label) = r.item()? else {
            return Err("COSE key: map labels must be integers".into());
        };
        if entries.iter().any(|(l, _)| *l == label) {
            return Err(format!("COSE key: duplicate label {label}"));
        }
        let value = r.item()?;
        entries.push((label, value));
    }
    if r.pos != bytes.len() {
        return Err("COSE key: trailing bytes after the key".into());
    }
    let get = |label: i128| entries.iter().find(|(l, _)| *l == label).map(|(_, v)| v);
    let int = |label: i128, name: &str| match get(label) {
        Some(Item::Int(v)) => Ok(*v),
        Some(_) => Err(format!("COSE key: '{name}' must be an integer")),
        None => Err(format!("COSE key: missing '{name}'")),
    };
    let bytes32 = |label: i128, name: &str| match get(label) {
        Some(Item::Bytes(b)) if b.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(b);
            Ok(out)
        }
        Some(_) => Err(format!("COSE key: '{name}' must be a 32-byte string")),
        None => Err(format!("COSE key: missing '{name}'")),
    };

    match (int(KTY, "kty")?, int(ALG, "alg")?, int(CRV, "crv")?) {
        (KTY_EC2, ALG_ES256, CRV_P256) => {
            let (x, y) = (bytes32(X, "x")?, bytes32(Y, "y")?);
            let mut sec1 = Vec::with_capacity(65);
            sec1.push(0x04);
            sec1.extend_from_slice(&x);
            sec1.extend_from_slice(&y);
            Ok(CoseKey::Es256 {
                sec1_uncompressed: sec1,
            })
        }
        (KTY_OKP, ALG_EDDSA, CRV_ED25519) => {
            if get(Y).is_some() {
                return Err("COSE key: an OKP key has no 'y'".into());
            }
            Ok(CoseKey::Ed25519 {
                public: bytes32(X, "x")?,
            })
        }
        (kty, alg, crv) => Err(format!(
            "COSE key: unsupported kty/alg/crv {kty}/{alg}/{crv} \
             (supported: EC2/ES256/P-256 and OKP/EdDSA/Ed25519)"
        )),
    }
}

/// Encode a `COSE_Key` (test helper and a reference for registration tooling).
#[cfg(test)]
pub(crate) fn encode(key: &CoseKey) -> Vec<u8> {
    fn head(out: &mut Vec<u8>, major: u8, arg: usize) {
        let arg = u8::try_from(arg).expect("small");
        if arg < 24 {
            out.push((major << 5) | arg);
        } else {
            out.push((major << 5) | 24);
            out.push(arg);
        }
    }
    fn int(out: &mut Vec<u8>, v: i64) {
        if v >= 0 {
            head(out, 0, usize::try_from(v).expect("small"));
        } else {
            head(out, 1, usize::try_from(-1 - v).expect("small"));
        }
    }
    fn bytes(out: &mut Vec<u8>, b: &[u8]) {
        head(out, 2, b.len());
        out.extend_from_slice(b);
    }
    let mut out = Vec::new();
    match key {
        CoseKey::Es256 { sec1_uncompressed } => {
            head(&mut out, 5, 5);
            int(&mut out, 1);
            int(&mut out, 2);
            int(&mut out, 3);
            int(&mut out, -7);
            int(&mut out, -1);
            int(&mut out, 1);
            int(&mut out, -2);
            bytes(&mut out, &sec1_uncompressed[1..33]);
            int(&mut out, -3);
            bytes(&mut out, &sec1_uncompressed[33..65]);
        }
        CoseKey::Ed25519 { public } => {
            head(&mut out, 5, 4);
            int(&mut out, 1);
            int(&mut out, 1);
            int(&mut out, 3);
            int(&mut out, -8);
            int(&mut out, -1);
            int(&mut out, 6);
            int(&mut out, -2);
            bytes(&mut out, public);
        }
    }
    out
}
