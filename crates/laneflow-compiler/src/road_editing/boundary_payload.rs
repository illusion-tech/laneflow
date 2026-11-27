//! 编制端与来源读方共用的实体参数量化。浮点乘积落在半毫米时再核对精确残差。
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
        error_mm: u32,
    },
    Curb {
        ends: [[i32; 4]; 2],
        error_mm: u32,
    },
}

struct Quantizer {
    max_error_mm: f64,
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
                error_mm,
            } => {
                out[0] = role;
                out[1..25].copy_from_slice(
                    [
                        offset.to_le_bytes(),
                        length.to_le_bytes(),
                        radius.to_le_bytes(),
                        bottom.to_le_bytes(),
                        top.to_le_bytes(),
                        error_mm.to_le_bytes(),
                    ]
                    .as_flattened(),
                );
                Some((1, 25))
            }
            Self::Curb { ends, error_mm } => {
                for (chunk, v) in out[..32]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(ends.into_iter().flatten())
                {
                    chunk.copy_from_slice(&v.to_le_bytes());
                }
                out[32..].copy_from_slice(&error_mm.to_le_bytes());
                Some((2, 36))
            }
        }
    }
}

impl Quantizer {
    fn millimeters(&mut self, meters: f64, field: &'static str) -> Result<i64, &'static str> {
        let rounded = super::cut_stations::rounded_difference_mm(meters, 0.0).ok_or(field)?;
        let error = meters.mul_add(1000.0, -(rounded as f64)).abs();
        self.max_error_mm = self.max_error_mm.max(error);
        Ok(rounded)
    }

    fn signed(&mut self, v: f64, field: &'static str) -> Result<i32, &'static str> {
        i32::try_from(self.millimeters(v, field)?).map_err(|_| field)
    }

    fn positive(&mut self, v: f64, field: &'static str) -> Result<u32, &'static str> {
        let n: u32 = self.millimeters(v, field)?.try_into().map_err(|_| field)?;
        if n == 0 { Err(field) } else { Ok(n) }
    }

    fn error_mm(&self, authored: f64, field: &'static str) -> Result<u32, &'static str> {
        if !authored.is_finite() || authored < 0.0 {
            return Err(field);
        }
        let rounded = if self.max_error_mm == 0.0 {
            0.0
        } else {
            self.max_error_mm.next_up()
        };
        let sum = authored.mul_add(1000.0, rounded);
        let upper = if rounded == 0.0 {
            if authored.mul_add(1000.0, -sum) > 0.0 {
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
        let mut q = Quantizer { max_error_mm: 0.0 };
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
                    error_mm: q
                        .error_mm(parameter_error_meters, "guardrail.parameterErrorMeters")?,
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
                    if left < right
                        || e.left_offset_meters < e.right_offset_meters
                        || (left == right) != (e.left_offset_meters == e.right_offset_meters)
                    {
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
                    error_mm: q.error_mm(parameter_error_meters, "curb.parameterErrorMeters")?,
                }
            }
        })
    }
}
