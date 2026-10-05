//! Correctness sweep limits (DESIGN "Limits sweep").
use std::collections::BTreeSet;

fn small_primes(max: u64) -> Vec<u64> {
    (2..=max).filter(|&n| super::reference::is_prime(n)).collect()
}

pub fn default_sweep() -> Vec<u64> {
    let mut s: BTreeSet<u64> = BTreeSet::new();
    for v in [0, 1, 2, 3, 4, 5, 8, 9, 15, 25, 121, 169, 289, 361, 961, 1_000_000] {
        s.insert(v);
    }
    for (a, b) in [
        (31, 34),
        (63, 66),
        (95, 97),
        (127, 130),
        (255, 258),
        (1023, 1025),
        (1999, 2001),
        (4095, 4097),
        (999_983, 1_000_003),
    ] {
        s.extend(a..=b);
    }
    // 64 odd numbers per word means 128 per word; 64*32 words per thread block means 2048 numbers per
    // block of one bit each. Cover multiples +-1 of both.
    for k in 1..=16u64 {
        for base in [128 * k, 2048 * k] {
            s.extend([base - 1, base, base + 1]);
        }
    }
    for p in small_primes(997) {
        let q = p * p;
        s.extend([q - 1, q, q + 1]);
    }
    s.into_iter().collect()
}

/// Parse "1,2,5..9,100". Ranges are inclusive.
pub fn parse_limits(text: &str) -> Result<Vec<u64>, String> {
    let mut out = BTreeSet::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once("..") {
            let a: u64 = a.parse().map_err(|_| format!("bad limit {part}"))?;
            let b: u64 = b.parse().map_err(|_| format!("bad limit {part}"))?;
            if a > b {
                return Err(format!("bad range {part}"));
            }
            out.extend(a..=b);
        } else {
            out.insert(part.parse::<u64>().map_err(|_| format!("bad limit {part}"))?);
        }
    }
    if out.is_empty() {
        return Err("no limits given".into());
    }
    Ok(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_has_required_points() {
        let s = default_sweep();
        for v in [0, 1, 2, 9, 997 * 997, 997 * 997 + 1, 1_000_003, 2047, 2049] {
            assert!(s.contains(&v), "missing {v}");
        }
        assert!(!s.contains(&10_000_000));
    }

    #[test]
    fn parse() {
        assert_eq!(parse_limits("1,3..5").unwrap(), vec![1, 3, 4, 5]);
        assert!(parse_limits("x").is_err());
    }
}
