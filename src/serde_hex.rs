//! Serde adapter that (de)serializes `u32` as `"0x"`-prefixed hex
//! strings.
//!
//! Designed to be used via `#[serde(with = "crate::serde_hex::u32")]`
//! on `u32` fields whose JSON form should be human-friendly hex
//! literals like `"0x20030000"` (much less error-prone than decimals
//! when users hand-edit a config file).
//!
//! On the read path we accept both `"0x..."` and `"0X..."` forms as
//! well as bare decimal numbers (e.g. `0` survives a round-trip
//! through anything that turns numbers into strings naïvely).
//! On the write path we always emit `"0x"` lowercase prefix and
//! uppercase 8-digit zero-padded hex.

use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

pub fn serialize<S: Serializer>(value: &u32, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_str(&format!("0x{value:08X}"))
}

pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<u32, D::Error> {
    let s = String::deserialize(de)?;
    parse_u32(&s).map_err(D::Error::custom)
}

fn parse_u32(s: &str) -> Result<u32, String> {
    let trimmed = s.trim();
    let (digits, radix) = if let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        (rest, 16u32)
    } else {
        (trimmed, 10u32)
    };
    u32::from_str_radix(digits, radix).map_err(|e| format!("invalid u32 {s:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct Wrapper {
        #[serde(with = "super")]
        addr: u32,
    }

    #[test]
    fn serializes_with_uppercase_hex_and_lowercase_prefix() {
        let w = Wrapper { addr: 0x2003_0000 };
        let s = serde_json::to_string(&w).unwrap();
        assert_eq!(s, r#"{"addr":"0x20030000"}"#);
    }

    #[test]
    fn deserializes_hex_prefixed() {
        let w: Wrapper = serde_json::from_str(r#"{"addr":"0x20030000"}"#).unwrap();
        assert_eq!(w.addr, 0x2003_0000);
    }

    #[test]
    fn deserializes_uppercase_prefix() {
        let w: Wrapper = serde_json::from_str(r#"{"addr":"0X1000"}"#).unwrap();
        assert_eq!(w.addr, 0x1000);
    }

    #[test]
    fn deserializes_bare_decimal() {
        let w: Wrapper = serde_json::from_str(r#"{"addr":"4096"}"#).unwrap();
        assert_eq!(w.addr, 4096);
    }

    #[test]
    fn round_trips() {
        for v in [0u32, 1, 0xFFFF_FFFF, 0x2003_1234] {
            let w = Wrapper { addr: v };
            let s = serde_json::to_string(&w).unwrap();
            let back: Wrapper = serde_json::from_str(&s).unwrap();
            assert_eq!(w, back);
        }
    }

    #[test]
    fn rejects_garbage() {
        let err: Result<Wrapper, _> = serde_json::from_str(r#"{"addr":"not-a-number"}"#);
        assert!(err.is_err());
    }
}
