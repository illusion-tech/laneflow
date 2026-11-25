//! 编制端与来源读方共用的实体参数量化。浮点乘积落在半微米时再核对精确残差。
use super::model::{BoundaryPayloadInput, CurbEndInput};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum QuantizedBoundaryPayload {
    Rule {
        directions: u8,
        effect: u8,
    },
    Guardrail {
        role: u8,
        offset: i32,
        length: u32,
        radius: u32,
        bottom: i32,
        top: i32,
        error_nm: u32,
    },
    Curb {
        ends: [[i32; 4]; 2],
        error_nm: u32,
    },
}

struct Quantizer {
    max_error_um: f64,
}

impl QuantizedBoundaryPayload {
    pub(crate) fn obstacle_bytes(self, out: &mut [u8; 36]) -> Option<(u8, usize)> {
        match self {
            Self::Rule { .. } => None,
            Self::Guardrail {
                role,
                offset,
                length,
                radius,
                bottom,
                top,
                error_nm,
            } => {
                out[0] = role;
                out[1..25].copy_from_slice(
                    [
                        offset.to_le_bytes(),
                        length.to_le_bytes(),
                        radius.to_le_bytes(),
                        bottom.to_le_bytes(),
                        top.to_le_bytes(),
                        error_nm.to_le_bytes(),
                    ]
                    .as_flattened(),
                );
                Some((1, 25))
            }
            Self::Curb { ends, error_nm } => {
                for (chunk, v) in out[..32]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(ends.into_iter().flatten())
                {
                    chunk.copy_from_slice(&v.to_le_bytes());
                }
                out[32..].copy_from_slice(&error_nm.to_le_bytes());
                Some((2, 36))
            }
        }
    }
}

impl Quantizer {
    fn micrometers(&mut self, meters: f64, field: &'static str) -> Result<i64, &'static str> {
        let scaled = meters * 1_000_000.0;
        if !scaled.is_finite() || scaled.abs() > f64::from(u32::MAX) + 1.0 {
            return Err(field);
        }
        let residual = meters.mul_add(1_000_000.0, -scaled);
        let rounded = if scaled.fract().abs() == 0.5 && residual != 0.0 {
            if residual < 0.0 {
                scaled.floor()
            } else {
                scaled.ceil()
            }
        } else {
            scaled.round_ties_even()
        };
        let error = meters.mul_add(1_000_000.0, -rounded).abs();
        self.max_error_um = self.max_error_um.max(error);
        Ok(rounded as i64)
    }

    fn signed(&mut self, v: f64, field: &'static str) -> Result<i32, &'static str> {
        self.micrometers(v, field)?.try_into().map_err(|_| field)
    }

    fn positive(&mut self, v: f64, field: &'static str) -> Result<u32, &'static str> {
        let n: u32 = self.micrometers(v, field)?.try_into().map_err(|_| field)?;
        if n == 0 { Err(field) } else { Ok(n) }
    }

    fn error_nm(&self, authored: f64, field: &'static str) -> Result<u32, &'static str> {
        if !authored.is_finite() || authored < 0.0 {
            return Err(field);
        }
        let rounded = if self.max_error_um == 0.0 {
            0.0
        } else {
            (self.max_error_um.next_up() * 1000.0).next_up()
        };
        let sum = authored.mul_add(1e9, rounded);
        let upper = if rounded == 0.0 {
            if authored.mul_add(1e9, -sum) > 0.0 {
                sum.next_up()
            } else {
                sum
            }
        } else {
            sum.next_up()
        };
        if upper.ceil() > f64::from(u32::MAX) {
            return Err(field);
        }
        Ok(upper.ceil() as u32)
    }
}

impl BoundaryPayloadInput {
    pub(crate) fn quantize(self) -> Result<QuantizedBoundaryPayload, &'static str> {
        let mut q = Quantizer { max_error_um: 0.0 };
        Ok(match self {
            Self::CrossingRule { directions, effect } => {
                if !(1..=3).contains(&directions) {
                    return Err("crossingRule.directions");
                }
                if !(1..=2).contains(&effect) {
                    return Err("crossingRule.effect");
                }
                QuantizedBoundaryPayload::Rule { directions, effect }
            }
            Self::Guardrail {
                anchor_role,
                lateral_offset_meters,
                horizontal_length_meters,
                radius_meters,
                bottom_height_meters,
                top_height_meters,
                parameter_error_meters,
            } => {
                if !(1..=2).contains(&anchor_role) {
                    return Err("guardrail.anchorRole");
                }
                let offset = q.signed(lateral_offset_meters, "guardrail.lateralOffsetMeters")?;
                let length =
                    q.positive(horizontal_length_meters, "guardrail.horizontalLengthMeters")?;
                let radius = q.positive(radius_meters, "guardrail.radiusMeters")?;
                let bottom = q.signed(bottom_height_meters, "guardrail.bottomHeightMeters")?;
                let top = q.signed(top_height_meters, "guardrail.topHeightMeters")?;
                if top <= bottom {
                    return Err("guardrail.topHeightMeters");
                }
                QuantizedBoundaryPayload::Guardrail {
                    role: anchor_role,
                    offset,
                    length,
                    radius,
                    bottom,
                    top,
                    error_nm: q
                        .error_nm(parameter_error_meters, "guardrail.parameterErrorMeters")?,
                }
            }
            Self::Curb {
                low,
                high,
                parameter_error_meters,
            } => {
                let mut end = |e: CurbEndInput, fields: [&'static str; 4]| {
                    let left = q.signed(e.left_offset_meters, fields[0])?;
                    let right = q.signed(e.right_offset_meters, fields[1])?;
                    let bottom = q.signed(e.bottom_height_meters, fields[2])?;
                    let top = q.signed(e.top_height_meters, fields[3])?;
                    if left < right || e.left_offset_meters < e.right_offset_meters {
                        return Err(fields[1]);
                    }
                    if top <= bottom {
                        return Err(fields[3]);
                    }
                    Ok([left, right, bottom, top])
                };
                let ends = [
                    end(
                        low,
                        [
                            "curb.low.leftOffset",
                            "curb.low.rightOffset",
                            "curb.low.bottomHeight",
                            "curb.low.topHeight",
                        ],
                    )?,
                    end(
                        high,
                        [
                            "curb.high.leftOffset",
                            "curb.high.rightOffset",
                            "curb.high.bottomHeight",
                            "curb.high.topHeight",
                        ],
                    )?,
                ];
                QuantizedBoundaryPayload::Curb {
                    ends,
                    error_nm: q.error_nm(parameter_error_meters, "curb.parameterErrorMeters")?,
                }
            }
        })
    }
}
