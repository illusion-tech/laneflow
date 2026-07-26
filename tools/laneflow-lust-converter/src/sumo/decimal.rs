//! Exact decimal rationals for LuST coordinate transforms (§3.2).

use std::{cmp::Ordering, fmt, str::FromStr};

use crate::{Error, Result};

/// Exact decimal `digits / 10^scale` used before a single binary64 conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactDecimal {
    digits: i128,
    scale: u32,
}

impl ExactDecimal {
    /// Zero value.
    pub const fn zero() -> Self {
        Self {
            digits: 0,
            scale: 0,
        }
    }

    /// Subtract `rhs` in exact decimal arithmetic.
    pub fn checked_sub(self, rhs: Self) -> Result<Self> {
        let scale = self.scale.max(rhs.scale);
        let left = self.rescale(scale)?;
        let right = rhs.rescale(scale)?;
        let digits = left.checked_sub(right).ok_or_else(|| {
            Error::SumoModel("exact decimal subtraction overflowed i128".to_owned())
        })?;
        Ok(Self { digits, scale }.normalized())
    }

    /// Add `rhs` in exact decimal arithmetic.
    pub fn checked_add(self, rhs: Self) -> Result<Self> {
        let scale = self.scale.max(rhs.scale);
        let left = self.rescale(scale)?;
        let right = rhs.rescale(scale)?;
        let digits = left
            .checked_add(right)
            .ok_or_else(|| Error::SumoModel("exact decimal addition overflowed i128".to_owned()))?;
        Ok(Self { digits, scale }.normalized())
    }

    /// Exact midpoint `(self + other) / 2`.
    pub fn midpoint(self, other: Self) -> Result<Self> {
        let sum = self.checked_add(other)?;
        let scale = sum.scale + 1;
        let digits = sum.digits;
        // (a+b)/2 with one extra decimal place: digits stay, scale += 1 when even;
        // when odd, cannot represent in decimal without more scale — use digits*5 / 10^(scale+1)
        // Equivalent: value = digits / 10^sum.scale / 2 = digits / 10^(sum.scale+1) * 5
        let digits = digits
            .checked_mul(5)
            .ok_or_else(|| Error::SumoModel("exact decimal midpoint overflowed i128".to_owned()))?;
        Ok(Self { digits, scale }.normalized())
    }

    /// Convert with one IEEE-754 binary64 round-to-nearest, ties-to-even step.
    ///
    /// `-0.0` is normalized to `+0.0`.
    pub fn to_f64(self) -> Result<f64> {
        let text = self.to_plain_string();
        let value = f64::from_str(&text).map_err(|source| Error::SumoModel(format!(
            "failed to convert exact decimal {text:?} to binary64: {source}"
        )))?;
        if value == 0.0 {
            Ok(0.0)
        } else {
            Ok(value)
        }
    }

    /// Convert a non-negative exact decimal number of seconds into integer milliseconds.
    ///
    /// Fails unless the value is strictly positive and exactly representable in whole milliseconds.
    pub fn to_strict_positive_millis(self) -> Result<u64> {
        if self.digits <= 0 {
            return Err(Error::SumoModel(
                "duration/offset must be a strictly positive exact decimal".to_owned(),
            ));
        }
        let millis_digits = if self.scale <= 3 {
            let factor = pow10(3 - self.scale)?;
            self.digits.checked_mul(factor).ok_or_else(|| {
                Error::SumoModel("duration/offset millisecond conversion overflowed".to_owned())
            })?
        } else {
            let divisor = pow10(self.scale - 3)?;
            if self.digits % divisor != 0 {
                return Err(Error::SumoModel(format!(
                    "duration/offset {self} is not an exact integer number of milliseconds"
                )));
            }
            self.digits / divisor
        };
        u64::try_from(millis_digits).map_err(|_| {
            Error::SumoModel("duration/offset millisecond value does not fit in u64".to_owned())
        })
    }

    /// Convert a non-negative exact decimal number of seconds into integer milliseconds.
    ///
    /// Zero is allowed (controller offsets).
    pub fn to_non_negative_millis(self) -> Result<u64> {
        if self.digits < 0 {
            return Err(Error::SumoModel(
                "offset must not be negative".to_owned(),
            ));
        }
        if self.digits == 0 {
            return Ok(0);
        }
        self.to_strict_positive_millis()
    }

    /// Compare two exact decimals.
    pub fn cmp_decimal(self, other: Self) -> Ordering {
        let scale = self.scale.max(other.scale);
        let left = self.rescale(scale).unwrap_or(i128::MAX);
        let right = other.rescale(scale).unwrap_or(i128::MAX);
        left.cmp(&right)
    }

    /// `self < other`
    pub fn is_less_than(self, other: Self) -> bool {
        self.cmp_decimal(other) == Ordering::Less
    }

    /// `self >= other`
    pub fn is_greater_or_equal(self, other: Self) -> bool {
        matches!(self.cmp_decimal(other), Ordering::Greater | Ordering::Equal)
    }

    fn rescale(self, scale: u32) -> Result<i128> {
        match scale.cmp(&self.scale) {
            Ordering::Equal => Ok(self.digits),
            Ordering::Less => Err(Error::SumoModel(
                "internal exact decimal rescale requested a smaller scale".to_owned(),
            )),
            Ordering::Greater => {
                let delta = scale - self.scale;
                let factor = pow10(delta)?;
                self.digits.checked_mul(factor).ok_or_else(|| {
                    Error::SumoModel("exact decimal rescale overflowed i128".to_owned())
                })
            }
        }
    }

    fn normalized(self) -> Self {
        if self.digits == 0 {
            return Self::zero();
        }
        let mut digits = self.digits;
        let mut scale = self.scale;
        while scale > 0 && digits % 10 == 0 {
            digits /= 10;
            scale -= 1;
        }
        Self { digits, scale }
    }

    fn to_plain_string(self) -> String {
        if self.scale == 0 {
            return self.digits.to_string();
        }
        let negative = self.digits < 0;
        let mut digits = self.digits.unsigned_abs().to_string();
        while digits.len() <= self.scale as usize {
            digits.insert(0, '0');
        }
        let split = digits.len() - self.scale as usize;
        let mut text = String::new();
        if negative {
            text.push('-');
        }
        text.push_str(&digits[..split]);
        text.push('.');
        text.push_str(&digits[split..]);
        text
    }
}

impl FromStr for ExactDecimal {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(Error::SumoModel(
                "exact decimal token must not be empty".to_owned(),
            ));
        }
        let negative = trimmed.starts_with('-');
        let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
        if unsigned.is_empty() || !unsigned.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(Error::SumoModel(format!(
                "invalid exact decimal token {trimmed:?}"
            )));
        }
        let (int_part, frac_part) = match unsigned.split_once('.') {
            Some((int_part, frac_part)) => (int_part, frac_part),
            None => (unsigned, ""),
        };
        if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::SumoModel(format!(
                "invalid exact decimal integer part in {trimmed:?}"
            )));
        }
        if !frac_part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::SumoModel(format!(
                "invalid exact decimal fractional part in {trimmed:?}"
            )));
        }
        if frac_part.len() > 18 {
            return Err(Error::SumoModel(format!(
                "exact decimal fractional scale too large in {trimmed:?}"
            )));
        }
        let combined = format!("{int_part}{frac_part}");
        let mut digits: i128 = combined.parse().map_err(|_| {
            Error::SumoModel(format!("exact decimal magnitude overflow in {trimmed:?}"))
        })?;
        if negative {
            digits = -digits;
        }
        Ok(Self {
            digits,
            scale: frac_part.len() as u32,
        }
        .normalized())
    }
}

impl fmt::Display for ExactDecimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

fn pow10(exponent: u32) -> Result<i128> {
    10_i128
        .checked_pow(exponent)
        .ok_or_else(|| Error::SumoModel("exact decimal pow10 overflowed i128".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_lust_origin_subtraction_matches_contract() {
        let sx = ExactDecimal::from_str("6806.88").expect("sx");
        let sy = ExactDecimal::from_str("5727.52").expect("sy");
        let ox = ExactDecimal::from_str("6806.88").expect("ox");
        let oz = ExactDecimal::from_str("5727.52").expect("oz");
        assert_eq!(sx.checked_sub(ox).expect("x").to_f64().expect("xf"), 0.0);
        assert_eq!(sy.checked_sub(oz).expect("z").to_f64().expect("zf"), 0.0);
    }

    #[test]
    fn negative_zero_normalizes_to_positive_zero() {
        let value = ExactDecimal::from_str("-0.00")
            .expect("parse")
            .to_f64()
            .expect("f64");
        assert_eq!(value, 0.0);
        assert!(value.is_sign_positive());
    }
}
