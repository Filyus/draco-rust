use std::collections::BTreeMap;

use crate::{json::JsonRef, ComponentType, ValidationProfile};

pub(crate) const MESH_QUANTIZATION: &str = "KHR_mesh_quantization";

pub(crate) fn core_component_type(ty: ComponentType) -> bool {
    use ComponentType::{F32, I16, I8, U16, U32, U8};
    matches!(ty, I8 | U8 | I16 | U16 | U32 | F32)
}

pub(crate) fn invalid_attribute_semantic<'a>(
    names: impl IntoIterator<Item = &'a str>,
    profile: ValidationProfile,
) -> Option<&'a str> {
    let mut indices = BTreeMap::new();
    for name in names {
        let Some((prefix, suffix)) = name.split_once('_') else {
            continue;
        };
        if !matches!(prefix, "TEXCOORD" | "COLOR" | "JOINTS" | "WEIGHTS") {
            continue;
        }
        if suffix.is_empty()
            || suffix.len() > 1 && suffix.starts_with('0')
            || !suffix.bytes().all(|byte| byte.is_ascii_digit())
            || profile == ValidationProfile::Gltf21Draft && suffix.len() > 9
        {
            return Some(name);
        }
        let Ok(index) = suffix.parse::<usize>() else {
            return Some(name);
        };
        indices.insert((prefix, index), name);
    }
    indices.iter().find_map(|(&(prefix, index), &name)| {
        (index != 0 && !indices.contains_key(&(prefix, index - 1))).then_some(name)
    })
}

pub(crate) fn mesh_quantization(root: JsonRef<'_>) -> bool {
    ["extensionsUsed", "extensionsRequired"]
        .into_iter()
        .all(|field| {
            root.get(field)
                .and_then(JsonRef::as_array)
                .is_some_and(|names| {
                    names
                        .iter()
                        .any(|name| name.as_str() == Some(MESH_QUANTIZATION))
                })
        })
}

#[derive(Clone, Copy)]
pub(crate) enum Semantic {
    Position,
    Normal,
    Tangent,
    TexCoord,
    Color,
    Joints,
    Weights,
}

impl Semantic {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "POSITION" => Some(Self::Position),
            "NORMAL" => Some(Self::Normal),
            "TANGENT" => Some(Self::Tangent),
            _ => match name.split_once('_')?.0 {
                "TEXCOORD" => Some(Self::TexCoord),
                "COLOR" => Some(Self::Color),
                "JOINTS" => Some(Self::Joints),
                "WEIGHTS" => Some(Self::Weights),
                _ => None,
            },
        }
    }

    pub(crate) fn components(self, morph: bool) -> &'static [u8] {
        match self {
            Self::Position | Self::Normal => &[3],
            Self::Tangent if morph => &[3],
            Self::Tangent | Self::Joints | Self::Weights => &[4],
            Self::TexCoord => &[2],
            Self::Color => &[3, 4],
        }
    }

    pub(crate) fn allows(
        self,
        ty: ComponentType,
        normalized: bool,
        morph: bool,
        quantized: bool,
    ) -> bool {
        use ComponentType::{F32, I16, I8, U16, U8};

        let float = ty == F32 && !normalized;
        let signed = matches!(ty, I8 | I16);
        let unsigned = matches!(ty, U8 | U16);
        if morph {
            return match self {
                Self::Position => float || quantized && signed,
                Self::Normal | Self::Tangent => float || quantized && signed && normalized,
                Self::TexCoord => {
                    float || normalized && (signed || unsigned) || quantized && signed
                }
                Self::Color => float || normalized && (signed || unsigned),
                Self::Joints | Self::Weights => false,
            };
        }
        match self {
            Self::Position => float || quantized && (signed || unsigned),
            Self::Normal | Self::Tangent => float || quantized && signed && normalized,
            Self::TexCoord => float || unsigned && normalized || quantized && (signed || unsigned),
            Self::Color | Self::Weights => float || unsigned && normalized,
            Self::Joints => unsigned && !normalized,
        }
    }
}
