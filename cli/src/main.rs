use std::{collections::BTreeMap, env, fs, path::PathBuf, process::ExitCode, time::Duration};

use zed_http_runner::{
    execute, load_env_file, load_environment, merge_variable_sources, parse_document,
    render_response, resolve_request, select_request, EnvironmentOptions, Selection,
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
    let mut environment_variables = BTreeMap::new();
    let mut explicit_variables = BTreeMap::new();
    let mut environment = None;
    let mut use_default_environment = false;
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
            "--env-file" => {
                index += 1;
                let path = argument(&arguments, index, "--env-file")?;
                let loaded = load_env_file(std::path::Path::new(path))
                    .map_err(|error| error.to_string())?;
                environment_variables.extend(loaded);
            }
            "--environment" => {
                index += 1;
                environment = Some(argument(&arguments, index, "--environment")?.into());
            }
            "--use-default-environment" => use_default_environment = true,
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
    let selected_environment = load_environment(&EnvironmentOptions {
        request_path: PathBuf::from(path),
        environment,
        use_default: use_default_environment,
        project_root,
        config,
        private_config,
    })
    .map_err(|error| error.to_string())?;
    environment_variables = merge_variable_sources(
        selected_environment,
        environment_variables,
        explicit_variables,
    );

    let source = fs::read_to_string(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    let requests = parse_document(&source).map_err(|error| error.to_string())?;
    let request = select_request(&requests, selection).map_err(|error| error.to_string())?;
    let request =
        resolve_request(request, &environment_variables).map_err(|error| error.to_string())?;
    let response = execute(
        &request,
        Duration::from_secs(timeout_seconds),
        maximum_response_bytes,
    )
    .map_err(|error| error.to_string())?;
    Ok(render_response(&response, maximum_response_bytes))
}

fn argument<'a>(arguments: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn usage() -> String {
    "usage: zed-http <file.http> [--name NAME | --index INDEX | --line LINE] [--environment NAME | --use-default-environment] [--project-root PATH] [--config PATH] [--private-config PATH] [--env-file .env] [--var NAME=VALUE] [--timeout-seconds N] [--max-response-bytes N]".into()
}
