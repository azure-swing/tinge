//! Native OCIO grades. Processing spaces are explicit; graph edges stay canonical.
use anyhow::{Result, ensure};
use ocio_rs::{Config, Context, Processor, TransformDirection, transform::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdlStyle {
    Asc,
    #[default]
    NoClamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeGrade {
    /// OCIO owns each Look's process space and context-dependent dependencies.
    Look {
        looks: String,
        #[serde(default)]
        inverse: bool,
    },
    /// Native ASC v1.2 SOP/saturation, with an explicit signal encoding.
    Cdl {
        color_space: String,
        slope: [f64; 3],
        offset: [f64; 3],
        power: [f64; 3],
        saturation: f64,
        #[serde(default)]
        style: CdlStyle,
        #[serde(default)]
        inverse: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exchange: Option<Box<crate::cdl::CdlProvenance>>,
    },
    /// RGB affine transform; alpha cannot contribute to RGB or be modified.
    Matrix {
        color_space: String,
        matrix: [[f64; 3]; 3],
        offset: [f64; 3],
        #[serde(default)]
        inverse: bool,
    },
}

pub(crate) fn direction(inverse: bool) -> TransformDirection {
    if inverse {
        TransformDirection::Inverse
    } else {
        TransformDirection::Forward
    }
}

fn name(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && !value.contains('\0'),
        "OCIO grade name must be nonempty and contain no NUL"
    );
    Ok(())
}

impl NodeGrade {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Look { looks, .. } => name(looks)?,
            Self::Cdl {
                color_space,
                slope,
                offset,
                power,
                saturation,
                inverse,
                exchange,
                ..
            } => {
                name(color_space)?;
                if let Some(exchange) = exchange {
                    exchange.validate()?;
                }
                ensure!(
                    slope.iter().all(|v| v.is_finite() && *v >= 0.0),
                    "CDL slope must be finite and nonnegative"
                );
                ensure!(
                    offset.iter().all(|v| v.is_finite()),
                    "CDL offset must be finite"
                );
                ensure!(
                    power.iter().all(|v| v.is_finite() && *v > 0.0),
                    "CDL power must be finite and positive"
                );
                ensure!(
                    saturation.is_finite() && *saturation >= 0.0,
                    "CDL saturation must be finite and nonnegative"
                );
                ensure!(
                    !inverse || (slope.iter().all(|v| *v > 0.0) && *saturation > 0.0),
                    "inverse CDL requires positive slope and saturation"
                );
            }
            Self::Matrix {
                color_space,
                matrix,
                offset,
                ..
            } => {
                name(color_space)?;
                ensure!(
                    matrix.iter().flatten().chain(offset).all(|v| v.is_finite()),
                    "RGB matrix and offset must be finite"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn processor(
        &self,
        config: &Config,
        context: &Context,
        anchor: &str,
    ) -> Result<Processor> {
        self.validate()?;
        ensure!(
            !config
                .try_color_space(anchor)?
                .ok_or_else(|| anyhow::anyhow!("OCIO grade anchor does not exist"))?
                .is_data(),
            "OCIO grade anchor must be a color space, not data"
        );
        if let Self::Look { looks, inverse } = self {
            let transform = LookTransform::create()?;
            transform.set_src(anchor)?;
            transform.set_dst(anchor)?;
            transform.set_looks(looks)?;
            return Ok(config.processor_from_transform_with_context(
                context,
                &transform,
                direction(*inverse),
            )?);
        }
        let group = GroupTransform::create()?;
        let (space, inverse) = match self {
            Self::Cdl {
                color_space,
                inverse,
                ..
            }
            | Self::Matrix {
                color_space,
                inverse,
                ..
            } => (color_space, *inverse),
            Self::Look { .. } => unreachable!(),
        };
        let enter = ColorSpaceTransform::create()?;
        ensure!(
            !config
                .try_color_space(space)?
                .ok_or_else(|| anyhow::anyhow!("OCIO grade process space does not exist"))?
                .is_data(),
            "OCIO grade process space must be a color space, not data"
        );
        enter.set_src(anchor)?;
        enter.set_dst(space)?;
        // Data spaces are not an implicit escape from a color-managed chain.
        enter.try_set_data_bypass(false)?;
        group.append_transform(&enter)?;
        match self {
            Self::Cdl {
                slope,
                offset,
                power,
                saturation,
                style,
                ..
            } => {
                let transform = CDLTransform::create()?;
                transform.try_set_slope(slope)?;
                transform.try_set_offset(offset)?;
                transform.try_set_power(power)?;
                transform.try_set_sat(*saturation)?;
                transform.try_set_style(match style {
                    CdlStyle::Asc => ocio_rs::CDLStyle::Asc,
                    CdlStyle::NoClamp => ocio_rs::CDLStyle::NoClamp,
                })?;
                transform.try_set_direction(direction(inverse))?;
                group.append_transform(&transform)?;
            }
            Self::Matrix { matrix, offset, .. } => {
                let transform = MatrixTransform::create()?;
                let mut rgba = [0.0; 16];
                for r in 0..3 {
                    for c in 0..3 {
                        rgba[r * 4 + c] = matrix[r][c];
                    }
                }
                rgba[15] = 1.0;
                transform.set_matrix(&rgba)?;
                transform.set_offset(&[offset[0], offset[1], offset[2], 0.0])?;
                transform.try_set_direction(direction(inverse))?;
                group.append_transform(&transform)?;
            }
            Self::Look { .. } => unreachable!(),
        }
        let leave = ColorSpaceTransform::create()?;
        leave.set_src(space)?;
        leave.set_dst(anchor)?;
        leave.try_set_data_bypass(false)?;
        group.append_transform(&leave)?;
        Ok(config.processor_from_transform_with_context(
            context,
            &group,
            TransformDirection::Forward,
        )?)
    }
}
