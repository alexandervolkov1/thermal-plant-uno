// Bounded decimal conversion for UART tokens of at most 64 bytes.
// 416 bits cover a 64-digit significand, decimal scaling, and f32 midpoints.
// A significand needs at most 213 bits. After the decimal-order check, the
// denominator is at most 10^109 (363 bits). Multiplication by a midpoint's
// 25-bit mantissa needs at most 388 bits; shifting the numerator needs at most
// 363 bits. An overflowing positive shift on the right is therefore greater
// than the left, and can be handled without retaining the overflowing bits.
const LIMBS: usize = 26;

#[derive(Clone)]
struct Integer([u16; LIMBS]);

impl Integer {
    fn zero() -> Self {
        Self([0; LIMBS])
    }

    fn mul(&mut self, value: u16) {
        let mut carry = 0u32;
        for limb in &mut self.0 {
            let product = u32::from(*limb) * u32::from(value) + carry;
            *limb = product as u16;
            carry = product >> 16;
        }
    }

    fn add(&mut self, value: u16) {
        let mut carry = u32::from(value);
        for limb in &mut self.0 {
            let sum = u32::from(*limb) + carry;
            *limb = sum as u16;
            carry = sum >> 16;
        }
    }

    fn shift(&mut self, bits: u16) -> bool {
        // Callers shift by at most 150 bits. Traverse backwards to shift in place.
        let words = usize::from(bits / 16);
        let part = bits % 16;
        let overflow = self.0[LIMBS - words..].iter().any(|limb| *limb != 0)
            || (part != 0 && self.0[LIMBS - words - 1] >> (16 - part) != 0);
        for destination in (0..LIMBS).rev() {
            self.0[destination] = if destination < words {
                0
            } else {
                let source = destination - words;
                let lower = if part != 0 && source > 0 {
                    self.0[source - 1] >> (16 - part)
                } else {
                    0
                };
                (self.0[source] << part) | lower
            };
        }
        overflow
    }

    fn compare(&self, other: &Self) -> core::cmp::Ordering {
        for index in (0..LIMBS).rev() {
            let order = self.0[index].cmp(&other.0[index]);
            if order != core::cmp::Ordering::Equal {
                return order;
            }
        }
        core::cmp::Ordering::Equal
    }
}

fn binary_parts(bits: u32) -> (u32, i16) {
    let exponent = ((bits >> 23) & 255) as i16;
    let fraction = bits & 0x7fffff;
    if exponent == 0 {
        (fraction, -149)
    } else {
        (fraction | 0x800000, exponent - 150)
    }
}

// Compare an exact decimal rational with mantissa * 2^exponent.
#[inline(never)]
fn compare_binary(
    numerator: &Integer,
    denominator: &Integer,
    mantissa: u32,
    exponent: i16,
) -> core::cmp::Ordering {
    let mut right = denominator.clone();
    right.mul((mantissa >> 16) as u16);
    right.shift(16);
    let mut low = denominator.clone();
    low.mul(mantissa as u16);
    let mut carry = 0u32;
    for index in 0..LIMBS {
        let sum = u32::from(right.0[index]) + u32::from(low.0[index]) + carry;
        right.0[index] = sum as u16;
        carry = sum >> 16;
    }
    let mut left = numerator.clone();
    if exponent < 0 {
        left.shift((-exponent) as u16);
    } else if right.shift(exponent as u16) {
        return core::cmp::Ordering::Less;
    }
    left.compare(&right)
}

// Preserve exact integer input for room.capacity; decimal/exponent input rounds to nearest-even.
pub(crate) fn parse_decimal(bytes: &[u8], exact_integer: bool) -> Option<f32> {
    use core::cmp::Ordering;
    if bytes.is_empty() || bytes.len() > 64 {
        return None;
    }
    let negative = bytes[0] == b'-';
    let mut index = usize::from(negative || bytes[0] == b'+');
    let mut numerator = Integer::zero();
    let mut digits = 0i16;
    let mut fractional = 0i16;
    let mut significant = 0i16;
    let mut dot = false;
    let mut nonzero = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() {
            numerator.mul(10);
            numerator.add(u16::from(byte - b'0'));
            digits += 1;
            fractional += i16::from(dot);
            nonzero |= byte != b'0';
            significant += i16::from(nonzero);
        } else if byte == b'.' && !dot {
            dot = true;
        } else {
            break;
        }
        index += 1;
    }
    if digits == 0 {
        return None;
    }
    let integer_input = !dot && index == bytes.len();
    let mut exponent = 0i16;
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        index += 1;
        let exponent_negative = bytes.get(index) == Some(&b'-');
        if exponent_negative || bytes.get(index) == Some(&b'+') {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            exponent = (exponent * 10 + i16::from(bytes[index] - b'0')).min(1000);
            index += 1;
        }
        if index == start {
            return None;
        }
        if exponent_negative {
            exponent = -exponent;
        }
    }
    if index != bytes.len() {
        return None;
    }
    let sign = if negative { 0x80000000 } else { 0 };
    if !nonzero {
        return Some(f32::from_bits(sign));
    }
    exponent -= fractional;
    let order = significant + exponent - 1;
    if !(-46..=38).contains(&order) {
        return None;
    }
    let mut denominator = Integer::zero();
    denominator.add(1);
    for _ in 0..exponent.abs() {
        if exponent < 0 {
            denominator.mul(10);
        } else {
            numerator.mul(10);
        }
    }
    let mut lower = 0u32;
    let mut upper = 0x7f800000u32;
    while upper - lower > 1 {
        let middle = lower + (upper - lower) / 2;
        let (mantissa, binary_exponent) = binary_parts(middle);
        if compare_binary(&numerator, &denominator, mantissa, binary_exponent) == Ordering::Less {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let (mantissa, binary_exponent) = binary_parts(lower);
    let exact = compare_binary(&numerator, &denominator, mantissa, binary_exponent);
    if exact_integer && integer_input && exact != Ordering::Equal {
        return None;
    }
    let midpoint = compare_binary(
        &numerator,
        &denominator,
        2 * mantissa + 1,
        binary_exponent - 1,
    );
    let bits = if midpoint == Ordering::Greater || (midpoint == Ordering::Equal && lower & 1 != 0) {
        upper
    } else {
        lower
    };
    if bits == 0 || bits == 0x7f800000 {
        return None;
    }
    Some(f32::from_bits(bits | sign))
}

pub(crate) fn write_decimal<W: ufmt::uWrite + ?Sized>(
    writer: &mut W,
    value: f32,
) -> Result<(), W::Error> {
    // Defaults and parse_decimal admit only finite values into configuration.
    debug_assert!(value.is_finite());
    let bits = value.to_bits();
    let magnitude = bits & 0x7fffffff;
    if bits >> 31 != 0 {
        writer.write_str("-")?;
    }
    if magnitude == 0 {
        return writer.write_str("0");
    }
    // Keep the Stage 2 integer representation for exactly representable u32 values.
    let positive = f32::from_bits(magnitude);
    if positive < 4294967296.0 && positive as u32 as f32 == positive {
        return ufmt::uwrite!(writer, "{}", positive as u32);
    }
    let (mut mantissa, exponent) = binary_parts(magnitude);
    // The exact decimal coefficient of any binary32 requires at most 112 digits.
    let mut digits = [0u8; 113];
    let mut length = 0;
    while mantissa != 0 {
        digits[length] = (mantissa % 10) as u8;
        mantissa /= 10;
        length += 1;
    }
    let factor = if exponent < 0 { 5u8 } else { 2 };
    for _ in 0..exponent.abs() {
        let mut carry = 0u8;
        for digit in &mut digits[..length] {
            // Each digit is <= 9, so product is <= 49.
            let product = *digit * factor + carry;
            *digit = product % 10;
            carry = product / 10;
        }
        if carry != 0 {
            digits[length] = carry;
            length += 1;
        }
    }
    let mut decimal_exponent = length as i16 - 1 + exponent.min(0);
    let mut significand = 0u32;
    for index in 0..9 {
        let digit = if index < length {
            digits[length - 1 - index]
        } else {
            0
        };
        significand = significand * 10 + u32::from(digit);
    }
    if length > 9 {
        let next = digits[length - 10];
        let sticky = digits[..length - 10].iter().any(|digit| *digit != 0);
        if next > 5 || (next == 5 && (sticky || significand & 1 != 0)) {
            significand += 1;
        }
    }
    if significand == 1000000000 {
        significand /= 10;
        decimal_exponent += 1;
    }
    // Nine significant decimal digits round-trip every finite binary32 value.
    ufmt::uwrite!(writer, "{}", (significand / 100000000) as u8)?;
    let mut remainder = significand % 100000000;
    if remainder != 0 {
        writer.write_str(".")?;
        let mut divisor = 10000000;
        while remainder != 0 {
            ufmt::uwrite!(writer, "{}", (remainder / divisor) as u8)?;
            remainder %= divisor;
            divisor /= 10;
        }
    }
    ufmt::uwrite!(writer, "e{}", decimal_exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Output(std::string::String);
    impl ufmt::uWrite for Output {
        type Error = core::convert::Infallible;
        fn write_str(&mut self, text: &str) -> Result<(), Self::Error> {
            self.0.push_str(text);
            Ok(())
        }
    }

    #[test]
    fn decimal_round_trip() {
        let mut state = 12345u32;
        for index in 0..2500 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let bits = if index < 512 {
                ((index / 2) << 23) | (index & 1)
            } else {
                state
            };
            let value = f32::from_bits(bits);
            if !value.is_finite() {
                continue;
            }
            let mut output = Output::default();
            write_decimal(&mut output, value).unwrap();
            assert_eq!(
                output.0.parse::<f32>().unwrap().to_bits(),
                bits,
                "{}",
                output.0
            );
            assert_eq!(
                parse_decimal(output.0.as_bytes(), false).unwrap().to_bits(),
                bits,
                "{}",
                output.0
            );
        }
        for value in [
            f32::MAX,
            f32::MIN,
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::from_bits(0x7fffff),
            -0.0,
        ] {
            let mut output = Output::default();
            write_decimal(&mut output, value).unwrap();
            assert_eq!(
                parse_decimal(output.0.as_bytes(), false).unwrap().to_bits(),
                value.to_bits()
            );
        }
    }

    #[test]
    fn decimal_input_matches_reference() {
        for text in [
            "100",
            "0",
            "-10",
            "0.5",
            "20.25",
            "-10.75",
            "4e-10",
            "2.5e-7",
            "1e3",
            "1E+3",
            ".5",
            "1.",
            "+3",
            "-0",
            "1.000000059604644775390625",
            "1.000000178813934326171875",
            "7.006492321624085e-46",
            "7.006492321624086e-46",
            "3.4028235e38",
            "3.4028236e38",
        ] {
            let expected = text.parse::<f32>().unwrap();
            let actual = parse_decimal(text.as_bytes(), false);
            if expected.is_infinite() || (expected == 0.0 && text.starts_with('7')) {
                assert!(actual.is_none(), "{text}");
            } else {
                assert_eq!(actual.unwrap().to_bits(), expected.to_bits(), "{text}");
            }
        }
        for text in [
            "", "+", "-", ".", "1..2", "1e", "1e+", "1e-", "NaN", "Inf", "Infinity", "1e100",
            "1e-100", " 1", "1 ", "1,2",
        ] {
            assert!(parse_decimal(text.as_bytes(), false).is_none(), "{text}");
        }
        assert!(parse_decimal(b"16777217", true).is_none());
        assert_eq!(parse_decimal(b"16777218", true), Some(16777218.0));
    }

    #[test]
    fn decimal_rounding_matches_reference_for_long_and_generated_inputs() {
        for text in [
            "340282356779733661637539395458142568448",
            "340282356779733661637539395458142568447",
            "1.23456789012345678901234567890123456789012345678901234567890123",
            "12345678901234567890123456789012345678901234567890123456789e-100",
            "1.000000059604644775390625000000000000000000000000000000000001",
        ] {
            assert!(text.len() <= 64);
            let expected = text.parse::<f32>().unwrap();
            let actual = parse_decimal(text.as_bytes(), false);
            if expected.is_infinite() {
                assert!(actual.is_none(), "{text}");
            } else {
                assert_eq!(actual.unwrap().to_bits(), expected.to_bits(), "{text}");
            }
        }
        let mut state = 54321u32;
        for index in 0..1000 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let text = std::format!(
                "{}{}.{}e{}",
                if state & 1 == 0 { "-" } else { "" },
                state,
                index,
                (state % 150) as i16 - 110
            );
            let expected = text.parse::<f32>().unwrap();
            let actual = parse_decimal(text.as_bytes(), false);
            if !expected.is_finite() || expected == 0.0 {
                assert!(actual.is_none(), "{text}");
            } else {
                assert_eq!(actual.unwrap().to_bits(), expected.to_bits(), "{text}");
            }
        }
        assert!(parse_decimal(b"+16777217", true).is_none());
        assert!(parse_decimal(b"-16777217", true).is_none());
    }
}
