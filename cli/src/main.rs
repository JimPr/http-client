use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufReader},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use zed_http_runner::{
    execute, load_selected_environment, merge_variable_sources, parse_document, render_response,
    resolve_request, select_environment, select_request, EnvironmentOptions, Selection,
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_048_576;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("zed-http: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<String>) -> Result<String, String> {
    let Some(path) = arguments.first() else {
        return Err(usage());
    };
    let mut selection = Selection::Index(0);
    let mut timeout_seconds = DEFAULT_TIMEOUT_SECONDS;
    let mut maximum_response_bytes = DEFAULT_MAX_RESPONSE_BYTES;
    let mut explicit_variables = BTreeMap::new();
    let mut environment = None;
    let mut select_environment_interactively = false;
    let mut project_root = None;
    let mut config = None;
    let mut private_config = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--name" => {
                index += 1;
                selection = Selection::Name(argument(&arguments, index, "--name")?.into());
            }

            "--index" => {
                index += 1;
                selection = Selection::Index(
                    argument(&arguments, index, "--index")?
                        .parse()
                        .map_err(|_| "--index must be a zero-based integer".to_owned())?,
                );
            }
            "--line" => {
                index += 1;
                selection = Selection::Line(
                    argument(&arguments, index, "--line")?
                        .parse()
                        .map_err(|_| "--line must be a one-based integer".to_owned())?,
                );
            }
            "--timeout-seconds" => {
                index += 1;
                timeout_seconds = argument(&arguments, index, "--timeout-seconds")?
                    .parse()
                    .map_err(|_| "--timeout-seconds must be an integer".to_owned())?;
            }
            "--max-response-bytes" => {
                index += 1;
                maximum_response_bytes = argument(&arguments, index, "--max-response-bytes")?
                    .parse()
                    .map_err(|_| "--max-response-bytes must be an integer".to_owned())?;
            }
            "--var" => {
                index += 1;
                let assignment = argument(&arguments, index, "--var")?;
                let (name, value) = assignment
                    .split_once('=')
                    .ok_or_else(|| "--var expects NAME=VALUE".to_owned())?;
                explicit_variables.insert(name.to_owned(), value.to_owned());
            }
            "--environment" => {
                index += 1;
                environment = Some(argument(&arguments, index, "--environment")?.into());
            }
            "--select-environment" => select_environment_interactively = true,
            "--project-root" => {
                index += 1;
                project_root = Some(PathBuf::from(argument(&arguments, index, "--project-root")?));
            }
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(argument(&arguments, index, "--config")?));
            }
            "--private-config" => {
                index += 1;
                private_config = Some(PathBuf::from(argument(&arguments, index, "--private-config")?));
            }
            option => return Err(format!("unknown option `{option}`\n{}", usage())),
        }
        index += 1;
    }
    if select_environment_interactively && environment.is_some() {
        return Err("--select-environment cannot be used with --environment".to_owned());
    }
    let source = fs::read_to_string(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    let requests = parse_document(&source).map_err(|error| error.to_string())?;
    let request = select_request(&requests, selection).map_err(|error| error.to_string())?;
    let environment_options = EnvironmentOptions {
        request_path: PathBuf::from(path),
        environment,
        project_root,
        config,
        private_config,
    };
    let selected_environment = if select_environment_interactively {
        let mut input = BufReader::new(io::stdin().lock());
        let mut output = io::stderr().lock();
        select_environment(&environment_options, &mut input, &mut output)
    } else {
        load_selected_environment(&environment_options)
    }
    .map_err(|error| error.to_string())?;
    let environment_variables = merge_variable_sources(
        selected_environment.values.clone(),
        BTreeMap::new(),
        request.inline_variables.clone(),
        explicit_variables,
    );

    let request =
        resolve_request(request, &environment_variables).map_err(|error| error.to_string())?;
    let response = execute(
        &request,
        Duration::from_secs(timeout_seconds),
        maximum_response_bytes,
    )
    .map_err(|error| error.to_string())?;
    let environment_indicator = selected_environment
        .name
        .map(|name| format!("Environment: {name}\n"))
        .unwrap_or_default();
    Ok(format!(
        "{environment_indicator}{}",
        render_response(&response, maximum_response_bytes)
    ))
}

fn argument<'a>(arguments: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn usage() -> String {
    "usage: zed-http <file.http> [--name NAME | --index INDEX | --line LINE] [--environment NAME | --select-environment] [--project-root PATH] [--config PATH] [--private-config PATH] [--var NAME=VALUE] [--timeout-seconds N] [--max-response-bytes N]".into()
}
