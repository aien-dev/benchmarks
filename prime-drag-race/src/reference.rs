//! Independent reference: deterministic Miller-Rabin on every odd n <= limit.
//! Canonical bitmap: bit k of little-endian u64 words is set iff 2k+1 is prime,
//! for 0 <= k < odd_count = (limit+1)/2. Tail bits are zero. The prime 2 is implicit.

pub fn mulmod(a: u64, b: u64, m: u64) -> u64 {
    ((a as u128 * b as u128) % m as u128) as u64
}

pub fn powmod(mut b: u64, mut e: u64, m: u64) -> u64 {
    let mut r = 1u64 % m;
    b %= m;
    while e > 0 {
        if e & 1 == 1 {
            r = mulmod(r, b, m);
        }
        b = mulmod(b, b, m);
        e >>= 1;
    }
    r
}

const BASES_SMALL: [u64; 4] = [2, 3, 5, 7];
const BASES_TWELVE: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];

/// Deterministic Miller-Rabin. Bases 2,3,5,7 are exact below 3,215,031,751;
/// the first 12 primes are exact below 3.3e24 (covers all u64).
pub fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    let bases: &[u64] = if n < 3_215_031_751 { &BASES_SMALL } else { &BASES_TWELVE };
    for &p in bases {
        if n == p {
            return true;
        }
        if n % p == 0 {
            return false;
        }
    }
    let mut d = n - 1;
    let mut s = 0u32;
    while d % 2 == 0 {
        d /= 2;
        s += 1;
    }
    'outer: for &a in bases {
        let mut x = powmod(a, d, n);
        if x == 1 || x == n - 1 {
            continue;
        }
        for _ in 1..s {
            x = mulmod(x, x, n);
            if x == n - 1 {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

pub fn odd_count(limit: u64) -> u64 {
    (limit + 1) / 2
}

pub fn word_count(limit: u64) -> usize {
    odd_count(limit).div_ceil(64) as usize
}

pub fn byte_len(limit: u64) -> usize {
    word_count(limit) * 8
}

pub fn canonical_words(limit: u64) -> Vec<u64> {
    let mut w = vec![0u64; word_count(limit)];
    for k in 0..odd_count(limit) {
        if is_prime(2 * k + 1) {
            w[(k / 64) as usize] |= 1u64 << (k % 64);
        }
    }
    w
}

pub fn canonical_bytes(limit: u64) -> Vec<u8> {
    canonical_words(limit).iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// pi(limit) from a canonical bitmap: popcount + 1 for the implicit prime 2.
pub fn prime_count(bytes: &[u8], limit: u64) -> u64 {
    let pc: u64 = bytes.iter().map(|b| b.count_ones() as u64).sum();
    pc + u64::from(limit >= 2)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// The odd number 2k+1 for bit k (for tail garbage: the would-be number past the limit).
    pub odd: u64,
    pub tail: bool,
    pub expected_prime: bool,
}

impl std::fmt::Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.tail {
            write!(f, "tail bit set past the limit (odd number {})", self.odd)
        } else if self.expected_prime {
            write!(f, "odd number {} is prime but the bit is clear", self.odd)
        } else {
            write!(f, "odd number {} is composite or 1 but the bit is set", self.odd)
        }
    }
}

/// First differing bit between two equal-length bitmaps.
pub fn first_difference(expected: &[u8], got: &[u8], limit: u64) -> Option<Diff> {
    debug_assert_eq!(expected.len(), got.len());
    for (i, (e, g)) in expected.iter().zip(got.iter()).enumerate() {
        let x = e ^ g;
        if x != 0 {
            let k = i as u64 * 8 + x.trailing_zeros() as u64;
            return Some(Diff {
                odd: 2 * k + 1,
                tail: k >= odd_count(limit),
                expected_prime: (e >> (k % 8)) & 1 == 1,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(n: u64) -> bool {
        if n < 2 {
            return false;
        }
        let mut d = 2;
        while d * d <= n {
            if n % d == 0 {
                return false;
            }
            d += 1;
        }
        true
    }

    fn trial_bytes(limit: u64) -> Vec<u8> {
        let mut w = vec![0u64; word_count(limit)];
        for k in 0..odd_count(limit) {
            if trial(2 * k + 1) {
                w[(k / 64) as usize] |= 1u64 << (k % 64);
            }
        }
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn equals_trial_division_every_limit_to_2000() {
        for limit in 0..=2000 {
            assert_eq!(canonical_bytes(limit), trial_bytes(limit), "limit {limit}");
        }
    }

    #[test]
    fn equals_trial_division_at_1e5() {
        assert_eq!(canonical_bytes(100_000), trial_bytes(100_000));
    }

    fn pi(limit: u64) -> u64 {
        prime_count(&canonical_bytes(limit), limit)
    }

    #[test]
    fn pi_table() {
        for (l, c) in [(10, 4), (100, 25), (1_000, 168), (10_000, 1229), (100_000, 9592), (1_000_000, 78_498)] {
            assert_eq!(pi(l), c, "pi({l})");
        }
    }

    #[test]
    #[ignore = "slow in debug builds"]
    fn pi_1e7() {
        assert_eq!(pi(10_000_000), 664_579);
    }

    #[test]
    fn large_primes_use_twelve_bases() {
        // 3,215,031,751 itself is the classic strong pseudoprime to bases 2,3,5,7.
        assert!(!is_prime(3_215_031_751));
        assert!(is_prime(18_446_744_073_709_551_557)); // largest prime below 2^64
    }

    #[test]
    fn empty_and_tiny_limits_have_no_words() {
        assert_eq!(canonical_bytes(0).len(), 0);
        assert_eq!(canonical_bytes(1).len(), 8);
    }
}
