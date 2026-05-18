//! Pure formatting helpers.
//!
//! These have no I/O, no dependencies on `probe-rs`, and exist so the
//! rest of the crate can produce nice output without re-inventing the
//! wheel and so that we can unit-test them cheaply.

/// Format a byte count using the largest unit that yields a non-zero
/// integer.
///
/// Whole multiples of the unit are printed without a fractional part
/// (e.g. `4 KiB`), otherwise one decimal place is shown (`1.5 KiB`).
/// Anything below 1 KiB is reported as bytes.
pub fn human_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    fn fmt(n: u64, unit: u64, name: &str) -> String {
        if n.is_multiple_of(unit) {
            format!("{} {}", n / unit, name)
        } else {
            format!("{:.1} {}", n as f64 / unit as f64, name)
        }
    }

    if n >= GIB {
        fmt(n, GIB, "GiB")
    } else if n >= MIB {
        fmt(n, MIB, "MiB")
    } else if n >= KIB {
        fmt(n, KIB, "KiB")
    } else {
        format!("{n} B")
    }
}

/// Render a slice of bytes as space-separated lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        // Writing into a String is infallible.
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Render a slice of bytes as printable ASCII; non-printable bytes
/// become `.`.
pub fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_below_kib() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1), "1 B");
        assert_eq!(human_bytes(1023), "1023 B");
    }

    #[test]
    fn human_bytes_exact_units() {
        assert_eq!(human_bytes(1024), "1 KiB");
        assert_eq!(human_bytes(4 * 1024), "4 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1 MiB");
        assert_eq!(human_bytes(1024 * 1024 * 1024), "1 GiB");
    }

    #[test]
    fn human_bytes_fractional() {
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024 + 512 * 1024), "1.5 MiB");
    }

    #[test]
    fn hex_empty_and_simple() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x00, 0xff, 0xa5]), "00 ff a5");
    }

    #[test]
    fn ascii_printable_and_not() {
        assert_eq!(ascii(b"Hi!"), "Hi!");
        assert_eq!(ascii(&[0x00, b'A', 0x7f, b'z']), ".A.z");
    }
}
