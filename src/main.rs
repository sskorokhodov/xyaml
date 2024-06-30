use clap::Arg;
use clap::ArgAction;
use clap::Command;
use serde_yaml::Value;
use std::collections::HashMap;
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
    env_substitutions: Vec<String>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    exec: Option<PathBuf>,
    subst_args_from_env: bool,
    exec_args: Vec<String>,
}

fn wrap_at(s: &str, at: usize) -> String {
    let words = s.split(&[' ', '\t']).filter(|l| !l.is_empty());
    let mut wrapped = vec![];
    let mut line = String::new();
    for w in words {
        if !line.is_empty() && line.len() + w.len() >= at {
            wrapped.push(line);
            line = "".into()
        }
        line = line + w + " ";
    }
    wrapped.push(line);
    wrapped.join("\n")
}

fn wrap_help(s: &str) -> String {
    wrap_at(s, 70)
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
            Arg::new("env-substitutions")
                .long("env-subst")
                .value_name("VAR")
                .help("Repace <VAR> placeholder with its environment variable value")
                .long_help(wrap_help("Repace the placeholder with the name of <VAR> with the corresponding environment variable value. The env substitutions happen after the path replacements."))
                .action(ArgAction::Append)
                .num_args(1),
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
            Command::new("exec").args([
                Arg::new("subst-args-with-env")
                    .long("subst-args-with-env")
                    .help(wrap_help("Substitue the arguments with the corresponding environment variable values."))
                    .num_args(0),
                Arg::new("cmd")
                    .value_name("cmd")
                    .action(ArgAction::Append)
                    .help("The executable and its arguments")
                    .trailing_var_arg(true)
                    .num_args(0..),
            ]),
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
        env_substitutions: matches
            .get_many::<String>("env-substitutions")
            .unwrap_or_default()
            .map(Clone::clone)
            .collect(),
        output: matches.get_one::<PathBuf>("output").map(Clone::clone),
        input: matches.get_one::<PathBuf>("input").map(Clone::clone),
        exec: None,
        subst_args_from_env: false,
        exec_args: vec![],
    };
    if let Some(matches) = matches.subcommand_matches("exec") {
        let cmd: Vec<_> = matches
            .get_many::<String>("cmd")
            .unwrap()
            .map(Clone::clone)
            .collect();
        config.exec = Some(PathBuf::from(&cmd[0]));
        config.subst_args_from_env = matches.get_flag("subst-args-with-env");
        let mut exec_args: Vec<String> = cmd.into_iter().skip(1).collect();
        if config.subst_args_from_env {
            exec_args = substitute_exec_args(&exec_args);
        }
        config.exec_args = exec_args;
    }
    config
}

fn substitute_exec_args(args: &[String]) -> Vec<String> {
    let mut result = vec![];
    for a in args.iter() {
        let val = if a.starts_with("{{") && a.ends_with("}}") {
            let var = a[2..a.len() - 2].to_string();
            std::env::var(&var)
                .unwrap_or_else(|e| {
                    fail!(
                        "exec: Failed to read the referred env variable `{}`\nerror=`{e}`",
                        var
                    )
                })
                .to_string()
        } else {
            a.clone()
        };
        result.push(val);
    }
    result
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

    substitute_env(&mut yaml, &config.env_substitutions);

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

fn substitute_env(obj: &mut Value, vars: &[String]) {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|v| (format!("{{{{{}}}}}", v), v.clone()))
        .collect();
    do_substitute_env(obj, &vars)
}

fn do_substitute_env(obj: &mut Value, vars: &HashMap<String, String>) {
    if let Some(map) = obj.as_mapping_mut() {
        for (_, obj) in map.iter_mut() {
            do_substitute_env(obj, vars);
        }
    } else if let Some(seq) = obj.as_sequence_mut() {
        for obj in seq.iter_mut() {
            do_substitute_env(obj, vars);
        }
    } else if let Some(s) = obj.as_str() {
        if let Some(var) = vars.get(s) {
            let new_value = std::env::var(var).unwrap_or_else(|e| {
                fail!("Failed to read the referred env variable `{var}`\nerror=`{e}`")
            });
            *obj = serde_yaml::from_str(&new_value).unwrap_or_else(|e| {
                fail!("New value is not a valid YAML:\n  new_value=`{new_value}`\n  env_var=`{var}`\n  error=`{e}`")
            });
        }
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
