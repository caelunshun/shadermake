use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub subdirectories: Vec<String>,
    #[serde(default)]
    pub shaders: HashMap<String, Shader>,
}

impl Manifest {
    pub fn from_toml(toml: &str) -> anyhow::Result<Self> {
        let manifest: Manifest = toml::from_str(toml)?;
        Ok(manifest)
    }
}

#[derive(Debug, Deserialize)]
pub struct Shader {
    pub path: String,
    pub kind: ShaderKind,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderKind {
    Vertex,
    Fragment,
    Compute,
}

impl From<ShaderKind> for naga::ShaderStage {
    fn from(kind: ShaderKind) -> Self {
        match kind {
            ShaderKind::Vertex => naga::ShaderStage::Vertex,
            ShaderKind::Fragment => naga::ShaderStage::Fragment,
            ShaderKind::Compute => naga::ShaderStage::Compute,
        }
    }
}

impl From<ShaderKind> for shaderc::ShaderKind {
    fn from(kind: ShaderKind) -> Self {
        match kind {
            ShaderKind::Vertex => shaderc::ShaderKind::Vertex,
            ShaderKind::Fragment => shaderc::ShaderKind::Fragment,
            ShaderKind::Compute => shaderc::ShaderKind::Compute,
        }
    }
}
