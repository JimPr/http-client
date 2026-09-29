//! Core functionality for deliberately invoked `.http` requests.
//!
//! This crate never discovers or runs requests by itself. Callers parse a document,
//! choose a request explicitly, resolve its variables, then execute it.

use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use chrono::Utc;
use reqwest::{
    Method,
    blocking::Client,
    header::{HeaderName, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use thiserror::Error;

const MAX_REDIRECTS: usize = 10;
const SAVE_ATTEMPTS: u64 = 32;
static RESPONSE_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum HttpFileError {
    #[error("line {line}: unsupported HTTP method `{method}`")]
    UnsupportedMethod { line: usize, method: String },
    #[error("line {line}: expected `METHOD URL`")]
    InvalidRequestLine { line: usize },
    #[error("line {line}: invalid header; expected `Name: value`")]
    InvalidHeader { line: usize },
    #[error("line {line}: invalid inline variable declaration")]
    InvalidInlineVariableDeclaration { line: usize },
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
    #[error("response body could not be saved")]
    ResponseSave,
    #[error("environment configuration could not be read")]
    EnvironmentConfigurationRead,
    #[error("environment configuration is invalid")]
    InvalidEnvironmentConfiguration,
    #[error("environment configuration version 1 is no longer supported; migrate to version 2")]
    LegacyEnvironmentConfigurationVersion,
    #[error("unsupported environment configuration version")]
    UnsupportedEnvironmentConfigurationVersion,
    #[error("unknown environment `{0}`")]
    UnknownEnvironment(String),
    #[error("no environment configuration was found")]
    EnvironmentConfigurationNotFound,
    #[error("--select-environment cannot be used with --environment")]
    IncompatibleInteractiveEnvironmentSelector,
    #[error("environment selection was cancelled; no request was sent")]
    EnvironmentSelectionCancelled,
    #[error("invalid environment selection; enter a number from the displayed list")]
    InvalidEnvironmentSelection,
}

const ENVIRONMENT_CONFIG_NAME: &str = "http-client.environments.json";
const PRIVATE_ENVIRONMENT_CONFIG_NAME: &str = "http-client.environments.private.json";
#[derive(Debug, Clone)]
pub struct EnvironmentOptions {
    pub request_path: std::path::PathBuf,
    pub environment: Option<String>,
    pub project_root: Option<std::path::PathBuf>,
    pub config: Option<std::path::PathBuf>,
    pub private_config: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedEnvironment {
    pub name: Option<String>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnvironmentConfiguration {
    version: u32,
    environments: Vec<NamedEnvironment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedEnvironment {
    name: String,
    variables: BTreeMap<String, String>,
}

/// Returns the explicitly selected environment's values without mutating the process
/// environment. Without an explicit selector it intentionally returns no values.
pub fn load_environment(
    options: &EnvironmentOptions,
) -> Result<BTreeMap<String, String>, HttpFileError> {
    Ok(load_selected_environment(options)?.values)
}

/// Loads the explicitly selected environment. A missing configuration deliberately
/// preserves historical behavior when no explicit environment was requested.
pub fn load_selected_environment(
    options: &EnvironmentOptions,
) -> Result<SelectedEnvironment, HttpFileError> {
    let (public, private) = read_environment_configurations(options)?;
    if public.is_none() && private.is_none() {
        if options.environment.is_some() {
            return Err(HttpFileError::EnvironmentConfigurationNotFound);
        }
        return Ok(SelectedEnvironment {
            name: None,
            values: BTreeMap::new(),
        });
    }

    let source = environment_source(public.as_ref(), private.as_ref());
    let selected = options.environment.clone();
    match selected {
        Some(selected) => {
            selected_environment(&selected, source, public.as_ref(), private.as_ref())
        }
        None => Ok(SelectedEnvironment {
            name: None,
            values: BTreeMap::new(),
        }),
    }
}

/// Renders an ordered terminal list and reads one selection. It only reads stdin
/// when a configuration exists, so old projects continue to run without interaction.
pub fn select_environment<R: BufRead, W: Write>(
    options: &EnvironmentOptions,
    reader: &mut R,
    writer: &mut W,
) -> Result<SelectedEnvironment, HttpFileError> {
    if options.environment.is_some() {
        return Err(HttpFileError::IncompatibleInteractiveEnvironmentSelector);
    }
    let (public, private) = read_environment_configurations(options)?;
    let Some(source) = environment_source(public.as_ref(), private.as_ref()) else {
        return Ok(SelectedEnvironment {
            name: None,
            values: BTreeMap::new(),
        });
    };
    writeln!(writer, "Select environment:")
        .map_err(|_| HttpFileError::EnvironmentSelectionCancelled)?;
    for (index, environment) in source.environments.iter().enumerate() {
        let preselected = if index == 0 { " (preselected)" } else { "" };
        writeln!(writer, "  {}. {}{preselected}", index + 1, environment.name)
            .map_err(|_| HttpFileError::EnvironmentSelectionCancelled)?;
    }
    write!(writer, "Enter a number [1]: ")
        .map_err(|_| HttpFileError::EnvironmentSelectionCancelled)?;
    writer
        .flush()
        .map_err(|_| HttpFileError::EnvironmentSelectionCancelled)?;
    let mut input = String::new();
    if reader
        .read_line(&mut input)
        .map_err(|_| HttpFileError::EnvironmentSelectionCancelled)?
        == 0
    {
        return Err(HttpFileError::EnvironmentSelectionCancelled);
    }
    let selected_index = if input.trim().is_empty() {
        0
    } else {
        input
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .filter(|index| *index < source.environments.len())
            .ok_or(HttpFileError::InvalidEnvironmentSelection)?
    };
    let selected = &source.environments[selected_index].name;
    selected_environment(selected, Some(source), public.as_ref(), private.as_ref())
}

fn read_environment_configurations(
    options: &EnvironmentOptions,
) -> Result<
    (
        Option<EnvironmentConfiguration>,
        Option<EnvironmentConfiguration>,
    ),
    HttpFileError,
> {
    let (public_path, private_path) = configuration_paths(options);
    Ok((
        read_environment_configuration(public_path.as_deref())?,
        read_environment_configuration(private_path.as_deref())?,
    ))
}

fn environment_source<'a>(
    public: Option<&'a EnvironmentConfiguration>,
    private: Option<&'a EnvironmentConfiguration>,
) -> Option<&'a EnvironmentConfiguration> {
    public.or(private)
}

fn selected_environment(
    selected: &str,
    source: Option<&EnvironmentConfiguration>,
    public: Option<&EnvironmentConfiguration>,
    private: Option<&EnvironmentConfiguration>,
) -> Result<SelectedEnvironment, HttpFileError> {
    if !source.is_some_and(|configuration| {
        configuration
            .environments
            .iter()
            .any(|environment| environment.name == selected)
    }) {
        return Err(HttpFileError::UnknownEnvironment(selected.to_owned()));
    }
    let mut values = BTreeMap::new();
    for configuration in [public, private].into_iter().flatten() {
        if let Some(environment) = configuration
            .environments
            .iter()
            .find(|environment| environment.name == selected)
        {
            values.extend(environment.variables.clone());
        }
    }
    Ok(SelectedEnvironment {
        name: Some(selected.to_owned()),
        values,
    })
}

/// Combines configuration sources in increasing precedence. Process variables are
/// resolved later by `resolve_request`, maintaining compatibility for variables absent
/// from the public and private JSON files, inline declarations, and CLI overrides.
pub fn merge_variable_sources(
    public_environment: BTreeMap<String, String>,
    private_environment: BTreeMap<String, String>,
    inline: BTreeMap<String, String>,
    explicit: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    public_environment
        .into_iter()
        .chain(private_environment)
        .chain(inline)
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
    let Some(mut directory) = request_path
        .parent()
        .and_then(|path| path.canonicalize().ok())
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
    let source =
        fs::read_to_string(path).map_err(|_| HttpFileError::EnvironmentConfigurationRead)?;
    let document: serde_json::Value = serde_json::from_str(&source)
        .map_err(|_| HttpFileError::InvalidEnvironmentConfiguration)?;
    match document.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => return Err(HttpFileError::LegacyEnvironmentConfigurationVersion),
        Some(2) => {}
        Some(_) => return Err(HttpFileError::UnsupportedEnvironmentConfigurationVersion),
        None => return Err(HttpFileError::InvalidEnvironmentConfiguration),
    }
    let configuration: EnvironmentConfiguration = serde_json::from_str(&source)
        .map_err(|_| HttpFileError::InvalidEnvironmentConfiguration)?;
    debug_assert_eq!(configuration.version, 2);
    validate_environment_configuration(&configuration)?;
    Ok(Some(configuration))
}

fn validate_environment_configuration(
    configuration: &EnvironmentConfiguration,
) -> Result<(), HttpFileError> {
    if configuration.environments.is_empty()
        || configuration
            .environments
            .iter()
            .any(|environment| environment.name.trim().is_empty())
    {
        return Err(HttpFileError::InvalidEnvironmentConfiguration);
    }
    let unique_names = configuration
        .environments
        .iter()
        .map(|environment| environment.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    (unique_names.len() == configuration.environments.len())
        .then_some(())
        .ok_or(HttpFileError::InvalidEnvironmentConfiguration)?;
    Ok(())
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
    /// Inline declarations visible when this request begins. Values are literal.
    pub inline_variables: BTreeMap<String, String>,
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
    let mut inline_variables = BTreeMap::new();

    for (offset, line) in source.lines().enumerate() {
        let line_number = offset + 1;
        if line.trim_start().starts_with("###") {
            if let Some(request) = parse_block(&block, pending_name.take(), &mut inline_variables)?
            {
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
    if let Some(request) = parse_block(&block, pending_name, &mut inline_variables)? {
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
    inline_variables: &mut BTreeMap<String, String>,
) -> Result<Option<HttpRequest>, HttpFileError> {
    let mut start_index = 0;
    while start_index < lines.len() {
        let (line_number, line) = lines[start_index];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            start_index += 1;
            continue;
        }
        if trimmed.starts_with('@') {
            let (variable_name, value) = parse_inline_declaration(trimmed, line_number)?;
            inline_variables.insert(variable_name, value);
            start_index += 1;
            continue;
        }
        break;
    }
    if start_index == lines.len() {
        return Ok(None);
    }
    let (start_line, request_line) = lines[start_index];
    let request_inline_variables = inline_variables.clone();
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
        headers.push((
            header_name.trim().to_owned(),
            header_value.trim().to_owned(),
        ));
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
        inline_variables: request_inline_variables,
        method,
        url: url.to_owned(),
        headers,
        body,
        start_line,
        end_line,
    }))
}

fn parse_inline_declaration(
    line: &str,
    line_number: usize,
) -> Result<(String, String), HttpFileError> {
    let Some((name, value)) = line.strip_prefix('@').and_then(|line| line.split_once('=')) else {
        return Err(HttpFileError::InvalidInlineVariableDeclaration { line: line_number });
    };
    let name = name.trim();
    if name.is_empty()
        || !name.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, '_' | '.' | '$' | '-')
        })
    {
        return Err(HttpFileError::InvalidInlineVariableDeclaration { line: line_number });
    }
    Ok((name.to_owned(), value.trim().to_owned()))
}

pub fn select_request(
    requests: &[HttpRequest],
    selection: Selection,
) -> Result<&HttpRequest, HttpFileError> {
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
        inline_variables: request.inline_variables.clone(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedResponseBody {
    pub path: PathBuf,
    pub byte_count: usize,
    pub truncated: bool,
}

/// Persists the captured response bytes beneath the project-local Zed data directory.
/// The output path never incorporates server-controlled data.
pub fn save_response_body(
    response: &ResponseData,
    request: &HttpRequest,
    request_path: &Path,
    project_root: Option<&Path>,
) -> Result<SavedResponseBody, HttpFileError> {
    let root = response_storage_root(request_path, project_root)?;
    let storage_parent = root.join(".zed");
    fs::create_dir_all(&storage_parent).map_err(|_| HttpFileError::ResponseSave)?;
    let storage_parent = storage_parent
        .canonicalize()
        .map_err(|_| HttpFileError::ResponseSave)?;
    if !storage_parent.starts_with(&root) {
        return Err(HttpFileError::ResponseSave);
    }
    let directory = storage_parent.join("http-client");
    fs::create_dir_all(&directory).map_err(|_| HttpFileError::ResponseSave)?;
    let directory = directory
        .canonicalize()
        .map_err(|_| HttpFileError::ResponseSave)?;
    if !directory.starts_with(&root) {
        return Err(HttpFileError::ResponseSave);
    }
    restrict_directory_permissions(&directory).map_err(|_| HttpFileError::ResponseSave)?;

    let label = sanitize_file_component(
        request
            .name
            .as_deref()
            .unwrap_or(&format!("{:?}-line-{}", request.method, request.start_line)),
    );
    let extension = response_file_extension(
        response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str()),
    );
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let process_id = std::process::id();

    for _ in 0..SAVE_ATTEMPTS {
        let counter = RESPONSE_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "{timestamp}-{label}-{process_id}-{counter}.{extension}"
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if restrict_file_permissions(&file).is_err() {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(HttpFileError::ResponseSave);
                }
                if file.write_all(&response.body).is_err() || file.sync_all().is_err() {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(HttpFileError::ResponseSave);
                }
                return Ok(SavedResponseBody {
                    path,
                    byte_count: response.body.len(),
                    truncated: response.truncated,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(HttpFileError::ResponseSave),
        }
    }
    Err(HttpFileError::ResponseSave)
}

fn response_storage_root(
    request_path: &Path,
    project_root: Option<&Path>,
) -> Result<PathBuf, HttpFileError> {
    let request_path = request_path
        .canonicalize()
        .map_err(|_| HttpFileError::ResponseSave)?;
    match project_root {
        Some(root) => {
            let root = root
                .canonicalize()
                .map_err(|_| HttpFileError::ResponseSave)?;
            if !request_path.starts_with(&root) {
                return Err(HttpFileError::ResponseSave);
            }
            Ok(root)
        }
        None => request_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or(HttpFileError::ResponseSave),
    }
}

fn response_file_extension(content_type: Option<&str>) -> &'static str {
    let media_type = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let Some(media_type) = media_type else {
        return "bin";
    };
    if media_type == "application/json" || media_type.ends_with("+json") {
        return "json";
    }
    if media_type == "application/xhtml+xml" {
        return "html";
    }
    if media_type == "application/xml" || media_type == "text/xml" || media_type.ends_with("+xml") {
        return "xml";
    }
    match media_type.as_str() {
        "text/html" => "html",
        "text/css" => "css",
        "application/javascript" | "text/javascript" => "js",
        "application/typescript" | "text/typescript" => "ts",
        "text/markdown" | "text/x-markdown" => "md",
        "application/yaml" | "application/x-yaml" | "text/yaml" | "text/x-yaml" => "yaml",
        "text/csv" => "csv",
        "application/graphql" | "application/x-graphql" => "graphql",
        "application/pdf" => "pdf",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" => "rar",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "application/wasm" => "wasm",
        value if value.starts_with("text/") => "txt",
        _ => "bin",
    }
}

fn sanitize_file_component(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '_',
        })
        .take(48)
        .collect();
    let value = value.trim_matches('_');
    if value.is_empty() {
        "response".into()
    } else {
        value.into()
    }
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(file: &fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_file_permissions(_: &fs::File) -> std::io::Result<()> {
    Ok(())
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
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or("<non-UTF-8>").into(),
            )
        })
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

#[cfg(test)]
mod response_storage_tests {
    use super::*;
    use std::{sync::Arc, thread};

    #[test]
    fn maps_normalized_and_unknown_content_types_to_the_closed_extension_set() {
        for (content_type, extension) in [
            (" Application/Problem+JSON ; charset=UTF-8 ", "json"),
            ("application/hal+xml; charset=utf-8", "xml"),
            ("application/xhtml+xml", "html"),
            ("text/unknown", "txt"),
            ("image/png", "png"),
            ("application/not-known", "bin"),
            ("not a media type", "bin"),
        ] {
            assert_eq!(response_file_extension(Some(content_type)), extension);
        }
        assert_eq!(response_file_extension(None), "bin");
    }

    #[test]
    fn saves_empty_truncated_bodies_and_uses_the_request_parent_without_a_root() {
        let directory = unique_test_directory("fallback");
        let request_path = directory.join("request.http");
        fs::write(&request_path, "GET http://example.test/").expect("request writes");
        let request = parse_document("GET http://example.test/")
            .expect("request parses")
            .remove(0);
        let response = ResponseData {
            final_url: "http://example.test/".into(),
            status: 204,
            duration_ms: 1,
            headers: vec![],
            body: vec![],
            truncated: true,
        };

        let saved =
            save_response_body(&response, &request, &request_path, None).expect("response saves");

        assert_eq!(saved.byte_count, 0);
        assert!(saved.truncated);
        assert_eq!(fs::read(saved.path).expect("body reads"), b"");
        fs::remove_dir_all(directory).expect("directory removes");
    }

    #[test]
    fn refuses_to_save_when_the_http_file_is_outside_the_supplied_root() {
        let base = unique_test_directory("confinement");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).expect("root creates");
        fs::create_dir_all(&outside).expect("outside creates");
        let request_path = outside.join("request.http");
        fs::write(&request_path, "GET http://example.test/").expect("request writes");
        let request = parse_document("GET http://example.test/")
            .expect("request parses")
            .remove(0);
        let response = test_response();

        let error = save_response_body(&response, &request, &request_path, Some(&root))
            .expect_err("outside request rejects");

        assert_eq!(error.to_string(), "response body could not be saved");
        assert!(!root.join(".zed/http-client").exists());
        fs::remove_dir_all(base).expect("directory removes");
    }

    #[test]
    fn reports_a_write_error_without_creating_a_body_when_the_storage_path_is_a_file() {
        let directory = unique_test_directory("write-error");
        let request_path = directory.join("request.http");
        fs::write(&request_path, "GET http://example.test/").expect("request writes");
        fs::write(directory.join(".zed"), "not a directory").expect("blocking file writes");
        let request = parse_document("GET http://example.test/")
            .expect("request parses")
            .remove(0);

        let error = save_response_body(&test_response(), &request, &request_path, Some(&directory))
            .expect_err("storage path rejects");

        assert_eq!(error.to_string(), "response body could not be saved");
        assert!(!directory.join(".zed/http-client").exists());
        fs::remove_dir_all(directory).expect("directory removes");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_storage_directory_that_escapes_the_project_root() {
        use std::os::unix::fs::symlink;

        let base = unique_test_directory("symlink");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).expect("root creates");
        fs::create_dir_all(&outside).expect("outside creates");
        symlink(&outside, root.join(".zed")).expect("symlink creates");
        let request_path = root.join("request.http");
        fs::write(&request_path, "GET http://example.test/").expect("request writes");
        let request = parse_document("GET http://example.test/")
            .expect("request parses")
            .remove(0);

        let error = save_response_body(&test_response(), &request, &request_path, Some(&root))
            .expect_err("escaping symlink rejects");

        assert_eq!(error.to_string(), "response body could not be saved");
        assert!(!outside.join("http-client").exists());
        fs::remove_dir_all(base).expect("directory removes");
    }

    #[test]
    fn concurrent_saves_use_distinct_files_without_changing_bytes() {
        let directory = unique_test_directory("concurrent");
        let request_path = directory.join("request.http");
        fs::write(&request_path, "GET http://example.test/").expect("request writes");
        let request = Arc::new(
            parse_document("GET http://example.test/")
                .expect("request parses")
                .remove(0),
        );
        let response = Arc::new(test_response());
        let first = {
            let request = Arc::clone(&request);
            let response = Arc::clone(&response);
            let request_path = request_path.clone();
            let directory = directory.clone();
            thread::spawn(move || {
                save_response_body(&response, &request, &request_path, Some(&directory))
            })
        };
        let second = {
            let request = Arc::clone(&request);
            let response = Arc::clone(&response);
            let request_path = request_path.clone();
            let directory = directory.clone();
            thread::spawn(move || {
                save_response_body(&response, &request, &request_path, Some(&directory))
            })
        };

        let first = first.join().expect("first thread").expect("first saves");
        let second = second.join().expect("second thread").expect("second saves");
        assert_ne!(first.path, second.path);
        assert_eq!(fs::read(first.path).expect("first reads"), [0, 255, 42]);
        assert_eq!(fs::read(second.path).expect("second reads"), [0, 255, 42]);
        fs::remove_dir_all(directory).expect("directory removes");
    }

    fn test_response() -> ResponseData {
        ResponseData {
            final_url: "http://example.test/".into(),
            status: 200,
            duration_ms: 1,
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            body: vec![0, 255, 42],
            truncated: false,
        }
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "zed-http-runner-response-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("directory creates");
        path
    }
}
