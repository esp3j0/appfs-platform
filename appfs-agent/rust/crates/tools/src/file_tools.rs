use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use commands::activate_conditional_skills_for_paths;
use runtime::{
    resolve_tool_path, resolve_tool_path_allow_missing, tool_output_root, EditFileOutput,
    ReadFileOutput, TextFilePayload, WriteFileOutput,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub(crate) const FILE_UNCHANGED_STUB: &str =
    "File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current; refer to that instead of re-reading.";

const READ_BEFORE_WRITE_ERROR: &str =
    "File has not been read yet. Read it first before writing to it.";
const FILE_UNEXPECTEDLY_MODIFIED_ERROR: &str =
    "File has been modified since read, either by the user or by a linter. Read it again before attempting to write it.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileUnchangedPayload {
    #[serde(rename = "filePath")]
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ReadToolOutput {
    Text { file: TextFilePayload },
    FileUnchanged { file: FileUnchangedPayload },
}

impl From<ReadFileOutput> for ReadToolOutput {
    fn from(value: ReadFileOutput) -> Self {
        Self::Text { file: value.file }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileToolStateSource {
    Read,
    WriteOrEdit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileToolState {
    timestamp_ms: u64,
    content: String,
    offset: Option<usize>,
    limit: Option<usize>,
    is_partial_view: bool,
    source: FileToolStateSource,
}

type ContextStateMap = HashMap<PathBuf, FileToolState>;
type GlobalStateMap = HashMap<PathBuf, ContextStateMap>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedRead {
    pub normalized_path: PathBuf,
    pub requested_offset: usize,
    pub limit: Option<usize>,
    pub dedup_output: Option<ReadToolOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedWrite {
    pub normalized_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedEdit {
    pub normalized_path: PathBuf,
    pub actual_old_string: String,
    pub actual_new_string: String,
}

fn global_file_tool_states() -> &'static Mutex<GlobalStateMap> {
    static STATES: OnceLock<Mutex<GlobalStateMap>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clean_state_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(stripped) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path
}

fn normalize_state_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return clean_state_path(canonical);
    }

    if let Some(parent) = path.parent() {
        let canonical_parent = parent
            .canonicalize()
            .map_or_else(|_| clean_state_path(parent.to_path_buf()), clean_state_path);
        if let Some(name) = path.file_name() {
            return clean_state_path(canonical_parent.join(name));
        }
    }

    clean_state_path(path.to_path_buf())
}

fn file_tool_context_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let session_root = tool_output_root(&cwd);
    let default_root = cwd.join(".claw");
    if session_root == default_root {
        Ok(PathBuf::from(format!(
            "thread-file-tool-state-{:?}",
            std::thread::current().id()
        )))
    } else {
        Ok(session_root)
    }
}

fn with_context_state_map<R>(action: impl FnOnce(&mut ContextStateMap) -> R) -> Result<R, String> {
    let context_root = file_tool_context_root()?;
    let mut states = global_file_tool_states()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let context_states = states.entry(context_root).or_default();
    Ok(action(context_states))
}

fn activate_skills_for_path(path: &Path) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    activate_conditional_skills_for_paths(&[path.to_path_buf()], &cwd)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn get_state_for_path(path: &Path) -> Result<Option<FileToolState>, String> {
    let normalized_path = normalize_state_path(path);
    with_context_state_map(|states| states.get(&normalized_path).cloned())
}

fn set_state_for_path(path: &Path, state: FileToolState) -> Result<(), String> {
    let normalized_path = normalize_state_path(path);
    with_context_state_map(|states| {
        states.insert(normalized_path, state);
    })
}

fn file_timestamp_ms(path: &Path) -> Result<u64, String> {
    let modified = fs::metadata(path)
        .map_err(|error| error.to_string())?
        .modified()
        .map_err(|error| error.to_string())?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

fn has_stale_file_contents(path: &Path, state: &FileToolState) -> Result<bool, String> {
    let last_write_time = file_timestamp_ms(path)?;
    if !state.is_partial_view {
        let current_content = fs::read_to_string(path).map_err(|error| error.to_string())?;
        return Ok(current_content != state.content);
    }

    Ok(last_write_time != state.timestamp_ms)
}

fn store_read_state(
    path: &Path,
    content: String,
    requested_offset: usize,
    limit: Option<usize>,
    source: FileToolStateSource,
) -> Result<(), String> {
    let is_full_view = limit.is_none() && matches!(requested_offset, 0 | 1);
    let timestamp_ms = file_timestamp_ms(path)?;
    set_state_for_path(
        path,
        FileToolState {
            timestamp_ms,
            content,
            offset: match source {
                FileToolStateSource::Read => Some(requested_offset),
                FileToolStateSource::WriteOrEdit => None,
            },
            limit: match source {
                FileToolStateSource::Read => limit,
                FileToolStateSource::WriteOrEdit => None,
            },
            is_partial_view: !is_full_view,
            source,
        },
    )
}

pub(crate) fn prepare_read(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<PreparedRead, String> {
    let normalized_path = resolve_tool_path(path).map_err(|error| error.to_string())?;
    activate_skills_for_path(&normalized_path)?;
    let requested_offset = offset.unwrap_or(1);

    let dedup_output = get_state_for_path(&normalized_path)?.and_then(|state| {
        if state.source != FileToolStateSource::Read
            || state.offset != Some(requested_offset)
            || state.limit != limit
        {
            return None;
        }

        let current_timestamp = file_timestamp_ms(&normalized_path).ok()?;
        if current_timestamp != state.timestamp_ms {
            return None;
        }

        Some(ReadToolOutput::FileUnchanged {
            file: FileUnchangedPayload {
                file_path: normalized_path.to_string_lossy().into_owned(),
            },
        })
    });

    Ok(PreparedRead {
        normalized_path,
        requested_offset,
        limit,
        dedup_output,
    })
}

pub(crate) fn record_read_result(
    path: &Path,
    output: &ReadFileOutput,
    requested_offset: usize,
    limit: Option<usize>,
) -> Result<(), String> {
    let is_full_view = limit.is_none() && matches!(requested_offset, 0 | 1);
    let content = if is_full_view {
        fs::read_to_string(path).map_err(|error| error.to_string())?
    } else {
        output.file.content.clone()
    };

    store_read_state(
        path,
        content,
        requested_offset,
        limit,
        FileToolStateSource::Read,
    )
}

pub(crate) fn prepare_write(path: &str) -> Result<PreparedWrite, String> {
    let normalized_path =
        resolve_tool_path_allow_missing(path).map_err(|error| error.to_string())?;
    activate_skills_for_path(&normalized_path)?;
    if !normalized_path.exists() {
        return Ok(PreparedWrite { normalized_path });
    }

    let state =
        get_state_for_path(&normalized_path)?.ok_or_else(|| READ_BEFORE_WRITE_ERROR.to_string())?;
    if state.is_partial_view {
        return Err(READ_BEFORE_WRITE_ERROR.to_string());
    }
    if has_stale_file_contents(&normalized_path, &state)? {
        return Err(FILE_UNEXPECTEDLY_MODIFIED_ERROR.to_string());
    }

    Ok(PreparedWrite { normalized_path })
}

pub(crate) fn record_write_result(path: &Path, output: &WriteFileOutput) -> Result<(), String> {
    store_read_state(
        path,
        output.content.clone(),
        1,
        None,
        FileToolStateSource::WriteOrEdit,
    )
}

pub(crate) fn prepare_edit(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<PreparedEdit, String> {
    if old_string == new_string {
        return Err(
            "No changes to make: old_string and new_string are exactly the same.".to_string(),
        );
    }

    let normalized_path =
        resolve_tool_path_allow_missing(path).map_err(|error| error.to_string())?;
    activate_skills_for_path(&normalized_path)?;
    let file_exists = normalized_path.exists();

    if !file_exists {
        if old_string.is_empty() {
            return Ok(PreparedEdit {
                normalized_path,
                actual_old_string: String::new(),
                actual_new_string: new_string.to_string(),
            });
        }
        return Err("File does not exist.".to_string());
    }

    if normalized_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ipynb"))
    {
        return Err(
            "File is a Jupyter Notebook. Use the NotebookEdit tool to edit this file.".to_string(),
        );
    }

    let file_content = fs::read_to_string(&normalized_path).map_err(|error| error.to_string())?;
    if old_string.is_empty() {
        if file_content.trim().is_empty() {
            return Ok(PreparedEdit {
                normalized_path,
                actual_old_string: String::new(),
                actual_new_string: new_string.to_string(),
            });
        }
        return Err("Cannot create new file - file already exists.".to_string());
    }

    let state =
        get_state_for_path(&normalized_path)?.ok_or_else(|| READ_BEFORE_WRITE_ERROR.to_string())?;
    if state.is_partial_view {
        return Err(READ_BEFORE_WRITE_ERROR.to_string());
    }
    if has_stale_file_contents(&normalized_path, &state)? {
        return Err(FILE_UNEXPECTEDLY_MODIFIED_ERROR.to_string());
    }

    let actual_old_string = find_actual_string(&file_content, old_string)
        .ok_or_else(|| format!("String to replace not found in file.\nString: {old_string}"))?;
    let matches = count_occurrences(&file_content, &actual_old_string);
    if matches > 1 && !replace_all {
        return Err(format!(
            "Found {matches} matches of the string to replace, but replace_all is false. To replace all occurrences, set replace_all to true. To replace only one occurrence, please provide more context to uniquely identify the instance.\nString: {old_string}"
        ));
    }

    Ok(PreparedEdit {
        normalized_path,
        actual_old_string: actual_old_string.clone(),
        actual_new_string: preserve_quote_style(old_string, &actual_old_string, new_string),
    })
}

pub(crate) fn record_edit_result(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    store_read_state(path, content, 1, None, FileToolStateSource::WriteOrEdit)
}

pub(crate) fn parse_read_tool_result(output: &str) -> Option<(ReadToolOutput, &str)> {
    parse_tool_result::<ReadToolOutput>(output).or_else(|| {
        parse_tool_result::<ReadFileOutput>(output)
            .map(|(result, trailing)| (ReadToolOutput::from(result), trailing))
    })
}

pub(crate) fn parse_write_tool_result(output: &str) -> Option<(WriteFileOutput, &str)> {
    parse_tool_result::<WriteFileOutput>(output)
}

pub(crate) fn parse_edit_tool_result(output: &str) -> Option<(EditFileOutput, &str)> {
    parse_tool_result::<EditFileOutput>(output)
}

pub(crate) fn read_tool_result_text(output: &ReadToolOutput) -> String {
    match output {
        ReadToolOutput::Text { file } => {
            if !file.content.is_empty() {
                format_file_lines(file)
            } else if file.total_lines == 0 {
                "<system-reminder>Warning: the file exists but the contents are empty.</system-reminder>".to_string()
            } else {
                format!(
                    "<system-reminder>Warning: the file exists but is shorter than the provided offset ({}). The file has {} lines.</system-reminder>",
                    file.start_line, file.total_lines
                )
            }
        }
        ReadToolOutput::FileUnchanged { .. } => FILE_UNCHANGED_STUB.to_string(),
    }
}

pub(crate) fn write_tool_result_text(output: &WriteFileOutput) -> String {
    match output.kind.as_str() {
        "create" => format!("File created successfully at: {}", output.file_path),
        _ => format!(
            "The file {} has been updated successfully.",
            output.file_path
        ),
    }
}

pub(crate) fn edit_tool_result_text(output: &EditFileOutput) -> String {
    let modified_note = if output.user_modified {
        ". The user modified your proposed changes before accepting them."
    } else {
        ""
    };

    if output.replace_all {
        format!(
            "The file {} has been updated{modified_note}. All occurrences were successfully replaced.",
            output.file_path
        )
    } else {
        format!(
            "The file {} has been updated successfully{modified_note}.",
            output.file_path
        )
    }
}

fn parse_tool_result<T: DeserializeOwned>(output: &str) -> Option<(T, &str)> {
    if let Ok(parsed) = serde_json::from_str::<T>(output) {
        return Some((parsed, ""));
    }

    let (json_prefix, trailing_text) = crate::split_json_prefix(output)?;
    let parsed = serde_json::from_str::<T>(json_prefix).ok()?;
    Some((parsed, trailing_text))
}

fn format_file_lines(file: &TextFilePayload) -> String {
    file.content
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            let line_number = file.start_line.saturating_add(index);
            format!("{line_number:>6}\t{line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

fn find_actual_string(file_content: &str, search_string: &str) -> Option<String> {
    if file_content.contains(search_string) {
        return Some(search_string.to_string());
    }

    let normalized_search = normalize_quotes(search_string);
    let normalized_file = normalize_quotes(file_content);
    let search_index = normalized_file.find(&normalized_search)?;
    Some(
        file_content
            .chars()
            .skip(search_index)
            .take(search_string.chars().count())
            .collect(),
    )
}

fn normalize_quotes(value: &str) -> String {
    value
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"")
}

fn preserve_quote_style(old_string: &str, actual_old_string: &str, new_string: &str) -> String {
    if old_string == actual_old_string {
        return new_string.to_string();
    }

    let has_double_quotes =
        actual_old_string.contains('\u{201C}') || actual_old_string.contains('\u{201D}');
    let has_single_quotes =
        actual_old_string.contains('\u{2018}') || actual_old_string.contains('\u{2019}');

    let mut result = new_string.to_string();
    if has_double_quotes {
        result = apply_curly_double_quotes(&result);
    }
    if has_single_quotes {
        result = apply_curly_single_quotes(&result);
    }

    result
}

fn is_opening_context(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }

    matches!(
        chars[index - 1],
        ' ' | '\t' | '\n' | '\r' | '(' | '[' | '{' | '\u{2014}' | '\u{2013}'
    )
}

fn apply_curly_double_quotes(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '"' {
            result.push(if is_opening_context(&chars, index) {
                '\u{201C}'
            } else {
                '\u{201D}'
            });
        } else {
            result.push(*ch);
        }
    }
    result
}

fn apply_curly_single_quotes(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '\'' {
            let prev = index.checked_sub(1).and_then(|idx| chars.get(idx));
            let next = chars.get(index + 1);
            let prev_is_letter = prev.is_some_and(|candidate| candidate.is_alphabetic());
            let next_is_letter = next.is_some_and(|candidate| candidate.is_alphabetic());
            if prev_is_letter && next_is_letter {
                result.push('\u{2019}');
            } else if is_opening_context(&chars, index) {
                result.push('\u{2018}');
            } else {
                result.push('\u{2019}');
            }
        } else {
            result.push(*ch);
        }
    }
    result
}
