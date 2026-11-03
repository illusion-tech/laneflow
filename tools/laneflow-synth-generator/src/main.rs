use std::path::Path;

use laneflow_synth_generator::{
    Bundle, Case, Error, Identity, LaneChangeParams, Provenance, generate, verify_directory,
};

const USAGE: &str = "用法：\n  laneflow-synth-generator generate <10000|100000> <W1|W2|W3|W4|LC> <16|33> <source-commit> <generator-commit> <new-directory> [clusters changers_per_cluster]\n  laneflow-synth-generator verify <directory>";

fn parse_scenario(
    arg: &str,
    clusters: Option<u32>,
    changers_per_cluster: Option<u32>,
) -> Result<Case, Error> {
    match arg {
        "W1" => Ok(Case::W1),
        "W2" => Ok(Case::W2),
        "W3" => Ok(Case::W3),
        "W4" => Ok(Case::W4),
        "LC" => {
            let params = LaneChangeParams {
                clusters: clusters.ok_or_else(|| Error::Invalid(USAGE.into()))?,
                changers_per_cluster: changers_per_cluster
                    .ok_or_else(|| Error::Invalid(USAGE.into()))?,
            };
            Ok(Case::LaneChange(params))
        }
        _ => Err(Error::Invalid(USAGE.into())),
    }
}

fn build(
    scale: &str,
    scenario: Case,
    delta: &str,
    source: &str,
    generator: &str,
    output: &str,
) -> Result<Bundle, Error> {
    let identity = Identity::new(
        scale.parse().map_err(|_| Error::Invalid(USAGE.into()))?,
        scenario,
        delta.parse().map_err(|_| Error::Invalid(USAGE.into()))?,
    )?;
    if Path::new(output).exists() {
        return Err(Error::Invalid("输出目录已存在".into()));
    }
    let bundle = generate(
        identity,
        Provenance {
            source_commit: source.into(),
            generator_commit: generator.into(),
        },
    )?;
    bundle.write_new(Path::new(output))?;
    Ok(bundle)
}

fn run(args: &[String]) -> Result<(), Error> {
    let bundle = match args {
        [command, scale, scenario, delta, source, generator, output] if command == "generate" => {
            build(
                scale,
                parse_scenario(scenario.as_str(), None, None)?,
                delta,
                source,
                generator,
                output,
            )?
        }
        [
            command,
            scale,
            scenario,
            delta,
            source,
            generator,
            output,
            clusters,
            changers,
        ] if command == "generate" => build(
            scale,
            parse_scenario(
                scenario.as_str(),
                Some(clusters.parse().map_err(|_| Error::Invalid(USAGE.into()))?),
                Some(changers.parse().map_err(|_| Error::Invalid(USAGE.into()))?),
            )?,
            delta,
            source,
            generator,
            output,
        )?,
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
