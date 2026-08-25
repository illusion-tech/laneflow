//! 编制/LIR 米制进入 LFCA 整数毫米与 `f32` 表面：先量化，再由调用方套闭包。

use laneflow_static_contract::{heading_f32_from_si, millimetres_from_si, millimetres_i32_from_si};

use super::PortableEmissionError;

pub(super) fn millimetres(meters: f64) -> Result<u32, PortableEmissionError> {
    millimetres_from_si(meters).ok_or(PortableEmissionError::InternalBindingMismatch)
}

pub(super) fn millimetres_i32(meters: f64) -> Result<i32, PortableEmissionError> {
    millimetres_i32_from_si(meters).ok_or(PortableEmissionError::InternalBindingMismatch)
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
    heading_f32_from_si(radians).ok_or(PortableEmissionError::InternalBindingMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_static_contract::{HEADING_MINUS_PI_F32_BITS, HEADING_PLUS_PI_F32_BITS};

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
