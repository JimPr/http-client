//! Core functionality for deliberately invoked `.http` requests.
//!
//! This crate never discovers or runs requests by itself. Callers parse a document,
//! choose a request explicitly, resolve its variables, then execute it.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use reqwest::{
    blocking::Client,
    header::{HeaderName, HeaderValue},
    redirect::Policy,
    Method,
};
use serde::Deserialize;
use thiserror::Error;

const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Error)]
pub enum HttpFileError {
    #[error("line {line}: unsupported HTTP method `{method}`")]
    UnsupportedMethod { line: usize, method: String },
    #[error("line {line}: expected `METHOD URL`")]
    InvalidRequestLine { line: usize },
    #[error("line {line}: invalid header; expected `Name: value`")]
    InvalidHeader { line: usize },
    #[error("no HTTP request was found")]
    NoRequests,
    #[error("no request named `{0}`")]
    RequestNameNotFound(String),
    #[error("request index {0} does not exist")]
    RequestIndexNotFound(usize),
    #[error("no request contains line {0}")]
    RequestLineNotFound(usize),
    #[error("variable `{0}` is not defined")]
    MissingVariable(String),
    #[error("invalid request header `{0}`")]
    InvalidHeaderName(String),
    #[error("invalid value for request header `{0}`")]
    InvalidHeaderValue(String),
    #[error("invalid request URL")]
    InvalidUrl,
    #[error("only http and https URLs are supported, got `{0}`")]
    UnsupportedUrlScheme(String),
    #[error("request failed (request details suppressed to protect secrets)")]
    RequestFailed,
    #[error("response body could not be read: {0}")]
    ResponseRead(#[from] std::io::Error),
    #[error("environment file could not be read: {0}")]
    EnvironmentFileRead(std::io::Error),
    #[error("environment file line {line}: expected NAME=VALUE")]
    InvalidEnvironmentLine { line: usize },
    #[error("environment configuration could not be read")]
    EnvironmentConfigurationRead,
    #[error("environment configuration is invalid")]
    InvalidEnvironmentConfiguration,
    #[error("unsupported environment configuration version")]
    UnsupportedEnvironmentConfigurationVersion,
    #[error("environment configuration with multiple environments requires defaultEnvironment")]
    DefaultEnvironmentRequired,
    #[error("environment configuration defaultEnvironment does not name an environment")]
    InvalidDefaultEnvironment,
    #[error("unknown environment `{0}`")]
    UnknownEnvironment(String),
    #[error("no environment configuration was found")]
    EnvironmentConfigurationNotFound,
    #[error("--environment and --use-default-environment cannot be used together")]
    IncompatibleEnvironmentSelectors,
}

const ENVIRONMENT_CONFIG_NAME: &str = "http-client.environments.json";
const PRIVATE_ENVIRONMENT_CONFIG_NAME: &str = "http-client.environments.private.json";

#[derive(Debug, Clone)]
pub struct EnvironmentOptions {
    pub request_path: std::path::PathBuf,
    pub environment: Option<String>,
    pub use_default: bool,
    pub project_root: Option<std::path::PathBuf>,
    pub config: Option<std::path::PathBuf>,
    pub private_config: Option<std::path::PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnvironmentConfiguration {
    version: u32,
    #[serde(default)]
    default_environment: Option<String>,
    environments: BTreeMap<String, BTreeMap<String, String>>,
}

/// Returns the selected environment's values without mutating the process environment.
/// When no selector is requested, and when `--use-default-environment` finds no
/// configuration, this intentionally returns no values to preserve historical behavior.
pub fn load_environment(
    options: &EnvironmentOptions,
) -> Result<BTreeMap<String, String>, HttpFileError> {
    if options.environment.is_some() && options.use_default {
        return Err(HttpFileError::IncompatibleEnvironmentSelectors);
    }
    if options.environment.is_none() && !options.use_default {
        return Ok(BTreeMap::new());
    }

    let (public_path, private_path) = configuration_paths(options);
    let public = read_environment_configuration(public_path.as_deref())?;
    let private = read_environment_configuration(private_path.as_deref())?;
    if public.is_none() && private.is_none() {
        return if options.use_default {
            Ok(BTreeMap::new())
        } else {
            Err(HttpFileError::EnvironmentConfigurationNotFound)
        };
    }

    let selected = match &options.environment {
        Some(name) => name.clone(),
        None => default_environment(public.as_ref(), private.as_ref())?,
    };
    let mut variables = BTreeMap::new();
    let mut found = false;
    for configuration in [public.as_ref(), private.as_ref()].into_iter().flatten() {
        if let Some(values) = configuration.environments.get(&selected) {
            variables.extend(values.clone());
            found = true;
        }
    }
    if found {
        Ok(variables)
    } else {
        Err(HttpFileError::UnknownEnvironment(selected))
    }
}

/// Combines explicit sources in increasing precedence. Process variables are resolved
/// later by `resolve_request`, maintaining compatibility for variables absent here.
pub fn merge_variable_sources(
    environment: BTreeMap<String, String>,
    env_file: BTreeMap<String, String>,
    explicit: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    environment
        .into_iter()
        .chain(env_file)
        .chain(explicit)
        .collect()
}

fn configuration_paths(
    options: &EnvironmentOptions,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    if options.config.is_some() || options.private_config.is_some() {
        return (options.config.clone(), options.private_config.clone());
    }
    discover_configuration_paths(&options.request_path, options.project_root.as_deref())
}

fn discover_configuration_paths(
    request_path: &Path,
    project_root: Option<&Path>,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let Some(mut directory) = request_path.parent().and_then(|path| path.canonicalize().ok())
    else {
        return (None, None);
    };
    let project_root = match project_root {
        Some(path) => match path.canonicalize() {
            Ok(path) => Some(path),
            Err(_) => return (None, None),
        },
        None => None,
    };
    if project_root
        .as_ref()
        .is_some_and(|root| !directory.starts_with(root))
    {
        return (None, None);
    }

    loop {
        let public = directory.join(ENVIRONMENT_CONFIG_NAME);
        let private = directory.join(PRIVATE_ENVIRONMENT_CONFIG_NAME);
        if public.is_file() || private.is_file() {
            return (
                public.is_file().then_some(public),
                private.is_file().then_some(private),
            );
        }
        if project_root.as_ref().is_some_and(|root| directory == *root) {
            break;
        }
        let Some(parent) = directory.parent() else {
            break;
        };
        directory = parent.to_path_buf();
    }
    (None, None)
}

fn read_environment_configuration(
    path: Option<&Path>,
) -> Result<Option<EnvironmentConfiguration>, HttpFileError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let source = fs::read_to_string(path).map_err(|_| HttpFileError::EnvironmentConfigurationRead)?;
    let configuration: EnvironmentConfiguration =
        serde_json::from_str(&source).map_err(|_| HttpFileError::InvalidEnvironmentConfiguration)?;
    if configuration.version != 1 {
        return Err(HttpFileError::UnsupportedEnvironmentConfigurationVersion);
    }
    validate_environment_configuration(&configuration)?;
    Ok(Some(configuration))
}

fn validate_environment_configuration(
    configuration: &EnvironmentConfiguration,
) -> Result<(), HttpFileError> {
    if configuration.environments.len() > 1 && configuration.default_environment.is_none() {
        return Err(HttpFileError::DefaultEnvironmentRequired);
    }
    if let Some(default) = &configuration.default_environment
        && !configuration.environments.contains_key(default)
    {
        return Err(HttpFileError::InvalidDefaultEnvironment);
    }
    Ok(())
}

fn default_environment(
    public: Option<&EnvironmentConfiguration>,
    private: Option<&EnvironmentConfiguration>,
) -> Result<String, HttpFileError> {
    if let Some(default) = public
        .and_then(|configuration| configuration.default_environment.clone())
        .or_else(|| private.and_then(|configuration| configuration.default_environment.clone()))
    {
        return Ok(default);
    }
    let names = [public, private]
        .into_iter()
        .flatten()
        .flat_map(|configuration| configuration.environments.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    (names.len() == 1)
        .then(|| names.into_iter().next().expect("one name exists"))
        .ok_or(HttpFileError::DefaultEnvironmentRequired)
}

/// Loads a simple dotenv file. File values are intentionally returned rather than
/// written to the process environment, so explicit CLI variables can take priority.
pub fn load_env_file(path: &Path) -> Result<BTreeMap<String, String>, HttpFileError> {
    let source = fs::read_to_string(path).map_err(HttpFileError::EnvironmentFileRead)?;
    let mut variables = BTreeMap::new();
    for (offset, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, raw_value)) = line.split_once('=') else {
            return Err(HttpFileError::InvalidEnvironmentLine { line: offset + 1 });
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(HttpFileError::InvalidEnvironmentLine { line: offset + 1 });
        }
        let value = raw_value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')))
            .unwrap_or(value);
        variables.insert(name.to_owned(), value.to_owned());
    }
    Ok(variables)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    fn parse(value: &str, line: usize) -> Result<Self, HttpFileError> {
        match value {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "PATCH" => Ok(Self::Patch),
            "DELETE" => Ok(Self::Delete),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            _ => Err(HttpFileError::UnsupportedMethod {
                line,
                method: value.into(),
            }),
        }
    }

    fn as_reqwest(self) -> Method {
        match self {
            Self::Get => Method::GET,
            Self::Post => Method::POST,
            Self::Put => Method::PUT,
            Self::Patch => Method::PATCH,
            Self::Delete => Method::DELETE,
            Self::Head => Method::HEAD,
            Self::Options => Method::OPTIONS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub name: Option<String>,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Name(String),
    Index(usize),
    /// One-based line number in the source document.
    Line(usize),
}

/// Parses request blocks separated by a line beginning with `###`.
pub fn parse_document(source: &str) -> Result<Vec<HttpRequest>, HttpFileError> {
    let mut requests = Vec::new();
    let mut pending_name = None;
    let mut block = Vec::new();

    for (offset, line) in source.lines().enumerate() {
        let line_number = offset + 1;
        if line.trim_start().starts_with("###") {
            if let Some(request) = parse_block(&block, pending_name.take())? {
                requests.push(request);
            }
            block.clear();
        } else {
            if let Some(name) = line.trim().strip_prefix("# @name ") {
                pending_name = Some(name.trim().to_owned());
            }
            block.push((line_number, line));
        }
    }
    if let Some(request) = parse_block(&block, pending_name)? {
        requests.push(request);
    }
    if requests.is_empty() {
        return Err(HttpFileError::NoRequests);
    }
    Ok(requests)
}

fn parse_block(
    lines: &[(usize, &str)],
    name: Option<String>,
) -> Result<Option<HttpRequest>, HttpFileError> {
    let Some(start_index) = lines.iter().position(|(_, line)| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    }) else {
        return Ok(None);
    };
    let (start_line, request_line) = lines[start_index];
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(HttpFileError::InvalidRequestLine { line: start_line })?;
    let url = parts
        .next()
        .ok_or(HttpFileError::InvalidRequestLine { line: start_line })?;
    let method = HttpMethod::parse(method, start_line)?;

    let mut headers = Vec::new();
    let mut cursor = start_index + 1;
    while cursor < lines.len() && !lines[cursor].1.trim().is_empty() {
        let (line_number, line) = lines[cursor];
        if line.trim_start().starts_with('#') {
            cursor += 1;
            continue;
        }
        let Some((header_name, header_value)) = line.split_once(':') else {
            return Err(HttpFileError::InvalidHeader { line: line_number });
        };
        headers.push((header_name.trim().to_owned(), header_value.trim().to_owned()));
        cursor += 1;
    }
    if cursor < lines.len() {
        cursor += 1;
    }
    let body_lines = &lines[cursor..];
    let body = (!body_lines.is_empty()).then(|| {
        body_lines
            .iter()
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n")
    });
    let end_line = lines.last().map_or(start_line, |(line, _)| *line);

    Ok(Some(HttpRequest {
        name,
        method,
        url: url.to_owned(),
        headers,
        body,
        start_line,
        end_line,
    }))
}

pub fn select_request<'a>(
    requests: &'a [HttpRequest],
    selection: Selection,
) -> Result<&'a HttpRequest, HttpFileError> {
    match selection {
        Selection::Name(name) => requests
            .iter()
            .find(|request| request.name.as_deref() == Some(name.as_str()))
            .ok_or(HttpFileError::RequestNameNotFound(name)),
        Selection::Index(index) => requests
            .get(index)
            .ok_or(HttpFileError::RequestIndexNotFound(index)),
        Selection::Line(line) => requests
            .iter()
            .find(|request| request.start_line <= line && line <= request.end_line)
            .ok_or(HttpFileError::RequestLineNotFound(line)),
    }
}

pub fn resolve_request(
    request: &HttpRequest,
    variables: &BTreeMap<String, String>,
) -> Result<HttpRequest, HttpFileError> {
    Ok(HttpRequest {
        name: request.name.clone(),
        method: request.method,
        url: resolve_text(&request.url, variables)?,
        headers: request
            .headers
            .iter()
            .map(|(name, value)| Ok((name.clone(), resolve_text(value, variables)?)))
            .collect::<Result<_, HttpFileError>>()?,
        body: request
            .body
            .as_deref()
            .map(|body| resolve_text(body, variables))
            .transpose()?,
        start_line: request.start_line,
        end_line: request.end_line,
    })
}

fn resolve_text(
    input: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String, HttpFileError> {
    let mut output = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return Err(HttpFileError::MissingVariable(after_start.to_owned()));
        };
        let name = after_start[..end].trim();
        let value = variables
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .ok_or_else(|| HttpFileError::MissingVariable(name.to_owned()))?;
        output.push_str(&value);
        rest = &after_start[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseData {
    pub final_url: String,
    pub status: u16,
    pub duration_ms: u128,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub truncated: bool,
}

/// Performs one explicitly selected request. TLS certificate validation remains enabled.
pub fn execute(
    request: &HttpRequest,
    timeout: Duration,
    maximum_response_bytes: usize,
) -> Result<ResponseData, HttpFileError> {
    let url = reqwest::Url::parse(&request.url).map_err(|_| HttpFileError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(HttpFileError::UnsupportedUrlScheme(url.scheme().to_owned()));
    }
    let client = Client::builder()
        .timeout(timeout)
        .redirect(Policy::limited(MAX_REDIRECTS))
        .build()
        .map_err(|_| HttpFileError::RequestFailed)?;
    let mut builder = client.request(request.method.as_reqwest(), url);
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| HttpFileError::InvalidHeaderName(name.clone()))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| HttpFileError::InvalidHeaderValue(name.to_string()))?;
        builder = builder.header(name, value);
    }
    if let Some(body) = &request.body {
        builder = builder.body(body.clone());
    }

    let started_at = Instant::now();
    let mut response = builder.send().map_err(|_| HttpFileError::RequestFailed)?;
    let duration_ms = started_at.elapsed().as_millis();
    let final_url = response.url().to_string();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("<non-UTF-8>").into()))
        .collect();
    let (body, truncated) = read_response_body(&mut response, maximum_response_bytes)?;

    Ok(ResponseData {
        final_url,
        status,
        duration_ms,
        headers,
        body,
        truncated,
    })
}

fn read_response_body(
    response: &mut reqwest::blocking::Response,
    maximum_response_bytes: usize,
) -> Result<(Vec<u8>, bool), HttpFileError> {
    let byte_limit = maximum_response_bytes.saturating_add(1) as u64;
    let mut limited = response.by_ref().take(byte_limit);
    let mut body = Vec::new();
    limited.read_to_end(&mut body)?;

    let truncated = body.len() > maximum_response_bytes;
    if truncated {
        body.truncate(maximum_response_bytes);
    }
    Ok((body, truncated))
}

pub fn render_response(response: &ResponseData, maximum_response_bytes: usize) -> String {
    let mut output = format!(
        "URL: {}\nStatus: {}\nDuration: {} ms\nSize: {} bytes{}\nHeaders:\n",
        response.final_url,
        response.status,
        response.duration_ms,
        response.body.len(),
        if response.truncated {
            format!(" (truncated at {maximum_response_bytes} bytes)")
        } else {
            String::new()
        }
    );
    for (name, value) in &response.headers {
        output.push_str(&format!("{name}: {value}\n"));
    }
    output.push_str("\nBody:\n");
    output.push_str(&render_body(&response.body));
    output
}

fn render_body(body: &[u8]) -> String {
    if body.contains(&0) {
        return binary_body_message(body.len());
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return binary_body_message(body.len());
    };

    let rendered = serde_json::from_str::<serde_json::Value>(text)
        .and_then(|json| serde_json::to_string_pretty(&json))
        .unwrap_or_else(|_| text.to_owned());
    format!("{rendered}\n")
}

fn binary_body_message(byte_count: usize) -> String {
    format!("<binary body omitted: {byte_count} bytes>\n")
}
