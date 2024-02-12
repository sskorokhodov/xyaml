use clap::Arg;
use clap::ArgAction;
use clap::Command;
use serde_yaml::Value;
use std::io::{self, Read};

#[macro_export]
macro_rules! fail {
    ( $msg: expr ) => {{
        eprintln!($msg);
        std::process::exit(1);
    }};
}

fn main() {
    let matches = Command::new("xyaml - YAML configuration trasnsformer")
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
                .num_args(1),
            Arg::new("output")
                .long("output")
                .value_name("FILE")
                .help("Write the result into the <FILE> instead of printing to <stdout>")
                .num_args(1),
            Arg::new("exec")
                .long("exec")
                .value_name("PATH")
                .help("Execute the binary at <PATH> after replacing the values")
                .num_args(1),
            Arg::new("exec-args")
                .long("exec-arg")
                .value_name("ARG")
                .action(ArgAction::Append)
                .help("The arguments to the executalbe")
                .num_args(1),
        ])
        .get_matches();

    let mut yaml_string = String::new();
    io::stdin()
        .read_to_string(&mut yaml_string)
        .expect("Failed to read from stdin");

    let mut yaml: Value =
        serde_yaml::from_str(&yaml_string).unwrap_or_else(|_| fail!("Failed to parse YAML"));

    let require_null = matches.get_flag("require-null");
    let env_values = matches.get_flag("env-values");
    let mut replacements = matches.get_many::<String>("replacements").unwrap();
    while let Some(path) = replacements.next() {
        let value = if env_values {
            let var = replacements.next().expect("no value for path");
            std::env::var(var)
                .unwrap_or_else(|_| fail!("the referred env variable `{var}` is not set"))
        } else {
            replacements.next().expect("no value for path").clone()
        };
        update_value(&mut yaml, path, &value, require_null);
    }

    let modified_yaml = serde_yaml::to_string(&yaml).expect("Failed to serialize YAML");
    println!("{}", modified_yaml);
}

fn update_value(obj: &mut Value, path: &str, new_value: &str, require_null: bool) {
    let segments: Value = serde_yaml::from_str(path)
        .unwrap_or_else(|_| fail!("Failed to parse the path as YAML:\n`{path}`"));
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
    *current_obj = serde_yaml::from_str(new_value).unwrap_or_else(|_| {
        fail!("New value is no a valid YAML:\n  new_value=`{new_value}`\n  path=`{path}`")
    });
}
