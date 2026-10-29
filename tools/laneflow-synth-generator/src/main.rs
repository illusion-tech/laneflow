use std::path::Path;

use laneflow_synth_generator::{Case, Error, Identity, Provenance, generate, verify_directory};

const USAGE: &str = "用法：\n  laneflow-synth-generator generate <10000|100000> <W1|W2|W3|W4> <16|33> <source-commit> <generator-commit> <new-directory>\n  laneflow-synth-generator verify <directory>";

fn run(args: &[String]) -> Result<(), Error> {
    let bundle = match args {
        [command, scale, case, delta, source, generator, output] if command == "generate" => {
            let case = match case.as_str() {
                "W1" => Case::W1,
                "W2" => Case::W2,
                "W3" => Case::W3,
                "W4" => Case::W4,
                _ => return Err(Error::Invalid(USAGE.into())),
            };
            let identity = Identity::new(
                scale.parse().map_err(|_| Error::Invalid(USAGE.into()))?,
                case,
                delta.parse().map_err(|_| Error::Invalid(USAGE.into()))?,
            )?;
            if Path::new(output).exists() {
                return Err(Error::Invalid("输出目录已存在".into()));
            }
            let bundle = generate(
                identity,
                Provenance {
                    source_commit: source.clone(),
                    generator_commit: generator.clone(),
                },
            )?;
            bundle.write_new(Path::new(output))?;
            bundle
        }
        [command, directory] if command == "verify" => verify_directory(Path::new(directory))?,
        _ => return Err(Error::Invalid(USAGE.into())),
    };
    println!("{}", serde_json::to_string(bundle.manifest())?);
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
    } else if let Err(error) = run(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
