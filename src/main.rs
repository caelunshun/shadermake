use std::path::PathBuf;

use argh::FromArgs;
use colored::Colorize;
use shadermake::Target;

#[derive(Debug, FromArgs)]
/// Build shaders according to `shadermake.toml` in the current directory.
struct CliOptions {
    #[argh(default = "PathBuf::from(\"target\".to_owned())")]
    #[argh(option)]
    /// the directory which will contain the compiled shaders
    target_dir: PathBuf,
    #[argh(option)]
    #[argh(default = "Target::Spirv")]
    /// the target shader kind to compile to
    target: Target,
}

struct Logger;

impl shadermake::Logger for Logger {
    fn on_shaders_gathered(&self, num_shaders: usize) {
        println!(
            "{} to compile {} shaders",
            "Ready".bright_blue(),
            num_shaders
        );
    }

    fn on_compiling(&self, shader: &str) {
        println!("{} {}", "Compiling".bright_green(), shader);
    }

    fn on_compile_error(&self, shader: &str, error: &dyn std::fmt::Display) {
        println!("{} while compiling {}: {}", "Error".red(), shader, error);
    }

    fn on_completed(&self) {
        println!("{}", "Finished".green());
    }
}

fn main() -> anyhow::Result<()> {
    let cli_args: CliOptions = argh::from_env();
    let options = shadermake::Options {
        source_dir: std::env::current_dir()?,
        target_dir: cli_args.target_dir,
        target: cli_args.target,
    };
    shadermake::build(&options, &Logger)
}
