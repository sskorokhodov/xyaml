use clap::Arg;
use clap::ArgAction;
use clap::Command;
use serde_yaml::Value;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::io::{self, Read};
use std::path::PathBuf;

#[macro_export]
macro_rules! fail {
    ( $($msg:expr),* ) => {{
        eprintln!($($msg),*);
        std::process::exit(1);
    }}
}

struct Config {
    require_null: bool,
    replacements: Vec<(String, String)>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    exec: Option<PathBuf>,
    exec_args: Vec<String>,
}

fn config() -> Config {
    let matches = Command::new("xyaml - YAML configuration transformer")
        .author("SUPREMATIC Technology Arts GmbH")
        .args([
            Arg::new("require-null")
                .long("require-null")
                .help("Require the replaced value to be `null`")
                .num_args(0),
            Arg::new("env-values")
                .long("env-values")
                .help("The values provided to `--set` are names of environment variables")
                .num_args(0),
            Arg::new("replacements")
                .long("set")
                .value_names(["PATH", "VALUE"])
                .help("Set the value at the specified path")
                .action(ArgAction::Append)
                .num_args(2),
            Arg::new("input")
                .long("input")
                .value_name("FILE")
                .help("Read the YAML from <FILE> instead of <stdin>")
                .value_parser(clap::value_parser!(PathBuf))
                .num_args(1),
            Arg::new("output")
                .long("output")
                .value_name("FILE")
                .help("Write the result into the <FILE> instead of printing to <stdout>")
                .value_parser(clap::value_parser!(PathBuf))
                .num_args(1),
        ])
        .subcommand(
            Command::new("exec").arg(
                Arg::new("cmd")
                    .value_name("cmd")
                    .action(ArgAction::Append)
                    .help("The executable and its arguments")
                    .trailing_var_arg(true)
                    .num_args(1..),
            ),
        )
        .get_matches();

    let env_values = matches.get_flag("env-values");
    let mut replacements: Vec<_> = matches
        .get_many::<String>("replacements")
        .unwrap_or_default()
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
        .collect();
    if env_values {
        for entry in replacements.iter_mut() {
            entry.1 = std::env::var(&entry.1).unwrap_or_else(|e| {
                fail!(
                    "Failed to read the referred env variable `{}`\nerror=`{e}`",
                    entry.1
                )
            });
        }
    }

    let mut config = Config {
        require_null: matches.get_flag("require-null"),
        replacements,
        output: matches.get_one::<PathBuf>("output").map(Clone::clone),
        input: matches.get_one::<PathBuf>("input").map(Clone::clone),
        exec: None,
        exec_args: vec![],
    };
    if let Some(matches) = matches.subcommand_matches("exec") {
        let cmd: Vec<_> = matches
            .get_many::<String>("cmd")
            .unwrap()
            .map(Clone::clone)
            .collect();
        config.exec = Some(PathBuf::from(&cmd[0]));
        config.exec_args = cmd.into_iter().skip(1).collect();
    }
    config
}

fn main() {
    let config = config();

    let yaml_string = if let Some(path) = config.input {
        let mut file = File::open(path.clone())
            .unwrap_or_else(|e| fail!("Failed to open the intput file `{path:?}`\nerror=`{e}`"));
        let mut yaml_string = String::new();
        file.read_to_string(&mut yaml_string)
            .unwrap_or_else(|e| fail!("Failed to read the intput file `{path:?}`\nerror=`{e}`"));
        yaml_string
    } else {
        let mut yaml_string = String::new();
        io::stdin()
            .read_to_string(&mut yaml_string)
            .expect("Failed to read from stdin");
        yaml_string
    };

    let mut yaml: Value =
        serde_yaml::from_str(&yaml_string).unwrap_or_else(|e| fail!("Failed to parse YAML: {e}"));

    for (path, value) in config.replacements.iter() {
        update_value(&mut yaml, path, value, config.require_null);
    }

    let modified_yaml = serde_yaml::to_string(&yaml).expect("Failed to serialize YAML");
    if let Some(path) = config.output {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(path.clone())
            .unwrap_or_else(|e| fail!("Failed to open the output file: {e}"));
        file.write_all(modified_yaml.as_bytes())
            .unwrap_or_else(|e| fail!("Failed to write the output file: {e}"));
    } else {
        println!("{}", modified_yaml);
    }

    if let Some(path) = config.exec {
        let mut cmd = std::process::Command::new(path);
        let cmd = cmd.args(&config.exec_args);
        let mut handle = cmd
            .spawn()
            .unwrap_or_else(|e| fail!("Failed to spawn the process:\ncmd=`{cmd:?}`\nerror=`{e}`"));
        handle.wait().ok();
    }
}

fn update_value(obj: &mut Value, path: &str, new_value: &str, require_null: bool) {
    let segments: Value = serde_yaml::from_str(path)
        .unwrap_or_else(|e| fail!("Failed to parse the path as YAML:\n`{path}`\nerror: {e}"));
    if !segments.is_sequence() {
        fail!("Path is not a YAML sequence:\n`{path}`")
    }
    let segments = segments.as_sequence().unwrap();
    let mut cursor = vec![];
    let mut current_obj = obj;
    for segment in segments.iter() {
        let segment_str = serde_yaml::to_string(segment)
            .unwrap()
            .trim_end_matches('\n')
            .to_string();
        cursor.push(segment_str.clone());

        if segment.is_sequence() {
            let seq = segment.as_sequence().unwrap();
            if seq.len() != 1 {
                fail!("Multiple sequence indexes are not supported\n  cursor=`{cursor:?}`\n  path=`{path}`");
            }
            let idx = seq.first().unwrap();
            if !idx.is_u64() {
                fail!("Invalid sequence index `{idx:?}`\n  cursor=`{cursor:?}`\n  path=`{path}`");
            }
            let idx = idx.as_u64().unwrap();
            current_obj = current_obj.get_mut(idx as usize).unwrap_or_else(|| {
                fail!("No entry at index {idx}\n  cursor=`{cursor:?}`\n  path=`{path}`")
            });
        } else {
            current_obj = current_obj.get_mut(segment).unwrap_or_else(|| {
                fail!("No key `{segment_str}`\n  cursor=`{cursor:?}`\n  path=`{path}`")
            });
        }
    }
    if require_null && !current_obj.is_null() {
        fail!("Object at path is not `null`:\n  obj={current_obj:?}\n  path=`{path}`");
    }
    *current_obj = serde_yaml::from_str(new_value).unwrap_or_else(|e| {
        fail!("New value is no a valid YAML:\n  new_value=`{new_value}`\n  path=`{path}`\n  error=`{e}`")
    });
}
