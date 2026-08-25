//! 编制/LIR 米制进入 LFCA v2 整数毫米与 `f32` 表面：先量化，再由调用方套闭包。

use laneflow_static_contract::{HEADING_MINUS_PI_F32_BITS, HEADING_PLUS_PI_F32_BITS};

use super::PortableEmissionError;

pub(super) fn millimetres(meters: f64) -> Result<u32, PortableEmissionError> {
    let mm = scaled_ties_even(meters, 1_000.0)?;
    u32::try_from(mm).map_err(|_| PortableEmissionError::InternalBindingMismatch)
}

pub(super) fn millimetres_i32(meters: f64) -> Result<i32, PortableEmissionError> {
    let mm = scaled_ties_even(meters, 1_000.0)?;
    i32::try_from(mm).map_err(|_| PortableEmissionError::InternalBindingMismatch)
}

pub(super) fn si_f32(value: f64) -> Result<f32, PortableEmissionError> {
    if !value.is_finite() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    let quantized = value as f32;
    if !quantized.is_finite() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(quantized)
}

pub(super) fn heading_f32(radians: f64) -> Result<f32, PortableEmissionError> {
    let quantized = si_f32(radians)?;
    if quantized.to_bits() == HEADING_PLUS_PI_F32_BITS {
        return Ok(f32::from_bits(HEADING_MINUS_PI_F32_BITS));
    }
    Ok(quantized)
}

fn scaled_ties_even(value: f64, scale: f64) -> Result<i64, PortableEmissionError> {
    if !value.is_finite() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    let scaled = (value * scale).round_ties_even();
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(scaled as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_vehicle_length_ties_to_even_then_closes() {
        assert_eq!(millimetres(0.099_6).unwrap(), 100);
        assert_eq!(millimetres(0.099_4).unwrap(), 99);
    }

    #[test]
    fn folds_plus_pi_heading_to_minus_pi() {
        assert_eq!(
            heading_f32(f64::from(f32::from_bits(HEADING_PLUS_PI_F32_BITS)))
                .unwrap()
                .to_bits(),
            HEADING_MINUS_PI_F32_BITS
        );
    }
}
