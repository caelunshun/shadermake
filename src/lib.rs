#![feature(or_patterns)]

use std::{
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{anyhow, Context};
use manifest::{Manifest, ShaderKind};
use naga::{
    back::spv::{Capability, WriterFlags},
    FastHashSet,
};
use rayon::prelude::*;

mod manifest;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Spirv,
    Wgsl,
    Glsl,
}

impl Target {
    pub fn extension(self) -> &'static str {
        match self {
            Target::Spirv => "spv",
            Target::Wgsl => "wgsl",
            Target::Glsl => "glsl",
        }
    }
}

impl FromStr for Target {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "spv" | "spirv" => Ok(Target::Spirv),
            "wgsl" => Ok(Target::Wgsl),
            "glsl" => Ok(Target::Glsl),
            s => Err(anyhow!(
                "invalid target '{}' (expected: spv, spirv, wgsl, glsl)",
                s
            )),
        }
    }
}

#[derive(Debug)]
pub struct Options {
    pub source_dir: PathBuf,
    pub target_dir: PathBuf,
    pub target: Target,
}

pub trait Logger: Send + Sync {
    fn on_shaders_gathered(&self, num_shaders: usize);

    fn on_compiling(&self, shader: &str);

    fn on_compile_error(&self, shader: &str, error: &dyn Display);

    fn on_completed(&self);
}

pub fn build(options: &Options, logger: &dyn Logger) -> anyhow::Result<()> {
    let shaders = gather_shaders(&options.source_dir)?;
    logger.on_shaders_gathered(shaders.0.len());

    let success = AtomicBool::new(true);

    shaders.0.into_par_iter().for_each(|shader| {
        logger.on_compiling(&shader.name);
        if let Err(e) = compile(&shader, options) {
            logger.on_compile_error(&shader.name, &format!("{:?}", e));
            success.store(false, Ordering::SeqCst);
        }
    });

    let exit_code = if success.load(Ordering::SeqCst) { 0 } else { 1 };
    std::process::exit(exit_code);
}

struct ShadersToCompile(Vec<ShaderToCompile>);

struct ShaderToCompile {
    name: String,
    path: PathBuf,
    kind: ShaderKind,
}

fn gather_shaders(source_dir: &Path) -> anyhow::Result<ShadersToCompile> {
    let mut queued_directories: Vec<PathBuf> = vec![source_dir.to_path_buf()];
    let mut shaders = Vec::new();

    while let Some(directory) = queued_directories.pop() {
        let manifest_path = directory.join("shadermake.toml");
        let manifest_string = fs::read_to_string(&manifest_path)
            .context("failed to read manifest file")
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest = Manifest::from_toml(&manifest_string)
            .context("failed to parse manifest")
            .with_context(|| format!("malformed manifest file {}", manifest_path.display()))?;

        for (shader_name, shader) in manifest.shaders {
            let shader = ShaderToCompile {
                name: shader_name,
                path: directory.join(&shader.path),
                kind: shader.kind,
            };
            shaders.push(shader);
        }

        for subdirectory in manifest.subdirectories {
            queued_directories.push(directory.join(&subdirectory));
        }
    }

    Ok(ShadersToCompile(shaders))
}

fn compile(shader: &ShaderToCompile, options: &Options) -> anyhow::Result<()> {
    let source_path = options.source_dir.join(&shader.path);
    let source = fs::read(&source_path)
        .with_context(|| format!("failed to read {}", source_path.display()))?;

    let output = compile_source(&source, shader, options)?;

    let base_path =
        pathdiff::diff_paths(&source_path, &options.source_dir).context("no base path")?;
    let mut target_path = options.target_dir.join(&base_path);
    target_path.set_extension(options.target.extension());
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    fs::write(&target_path, &output)?;

    Ok(())
}

fn compile_source(
    source: &[u8],
    shader: &ShaderToCompile,
    options: &Options,
) -> anyhow::Result<Vec<u8>> {
    let source_kind = ShaderSourceKind::guess(&shader.path).context(
        "failed to guess shader source kind from file extension (expected 'glsl' or 'wgsl')",
    )?;
    let compile_fn = compile_fn(source_kind, options.target)
        .context("failed to find a suitable compilation tool for shader")?;

    let result =
        compile_fn(source, shader.kind, options.target).context("failed to compile shader")?;
    Ok(result)
}

enum ShaderSourceKind {
    Wgsl,
    Glsl,
}

impl ShaderSourceKind {
    pub fn guess(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "wgsl" => Some(ShaderSourceKind::Wgsl),
            "glsl" => Some(ShaderSourceKind::Glsl),
            _ => None,
        }
    }
}

fn compile_fn(
    source_kind: ShaderSourceKind,
    target: Target,
) -> Option<fn(&[u8], ShaderKind, Target) -> anyhow::Result<Vec<u8>>> {
    match (source_kind, target) {
        (ShaderSourceKind::Wgsl, Target::Glsl | Target::Spirv) => Some(compile_naga_wgsl),
        (ShaderSourceKind::Wgsl, Target::Wgsl) => Some(compile_identity),
        (ShaderSourceKind::Glsl, Target::Spirv) => Some(compile_shaderc_glsl),
        (ShaderSourceKind::Glsl, Target::Wgsl) => None,
        (ShaderSourceKind::Glsl, Target::Glsl) => Some(compile_identity),
    }
}

fn compile_naga_wgsl(source: &[u8], kind: ShaderKind, target: Target) -> anyhow::Result<Vec<u8>> {
    let module = naga::front::wgsl::parse_str(std::str::from_utf8(source)?)
        .ok()
        .context("failed to parse WGSL")?;

    compile_naga(&module, kind, target)
}

fn compile_naga(
    module: &naga::Module,
    kind: ShaderKind,
    target: Target,
) -> anyhow::Result<Vec<u8>> {
    let stage = naga::ShaderStage::from(kind);

    match target {
        Target::Spirv => {
            let mut capabilities = FastHashSet::default();
            capabilities.insert(Capability::Shader);
            let u32s = naga::back::spv::write_vec(&module, WriterFlags::DEBUG, capabilities)?;
            Ok(bytemuck::cast_slice(&u32s).to_vec())
        }
        Target::Glsl => {
            let mut vec = Vec::new();
            let options = naga::back::glsl::Options {
                version: naga::back::glsl::Version::Desktop(450),
                entry_point: (stage, "main".to_owned()),
            };
            let mut writer = naga::back::glsl::Writer::new(&mut vec, &module, &options)?;
            writer.write()?;

            Ok(vec)
        }
        Target::Wgsl => unreachable!(),
    }
}

fn compile_identity(source: &[u8], _: ShaderKind, _: Target) -> anyhow::Result<Vec<u8>> {
    Ok(source.to_vec())
}

fn compile_shaderc_glsl(source: &[u8], kind: ShaderKind, _: Target) -> anyhow::Result<Vec<u8>> {
    let mut compiler = shaderc::Compiler::new().context("failed to create shaderc compiler")?;
    let source = std::str::from_utf8(source)?;
    let spirv = compiler.compile_into_spirv(source, kind.into(), "", "main", None)?;
    Ok(spirv.as_binary_u8().to_vec())
}
