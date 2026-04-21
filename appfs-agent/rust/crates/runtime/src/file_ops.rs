use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use glob::Pattern;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// Maximum file size that can be read (10 MB).
const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum file size that can be written (10 MB).
const MAX_WRITE_SIZE: usize = 10 * 1024 * 1024;

/// Check whether a file appears to contain binary content by examining
/// the first chunk for NUL bytes.
fn is_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 8192];
    let bytes_read = file.read(&mut buffer)?;
    Ok(buffer[..bytes_read].contains(&0))
}

/// Validate that a resolved path stays within the given workspace root.
/// Returns the canonical path on success, or an error if the path escapes
/// the workspace boundary (e.g. via `../` traversal or symlink).
#[allow(dead_code)]
fn validate_workspace_boundary(resolved: &Path, workspace_root: &Path) -> io::Result<()> {
    if !resolved.starts_with(workspace_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path {} escapes workspace boundary {}",
                resolved.display(),
                workspace_root.display()
            ),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextFilePayload {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "numLines")]
    pub num_lines: usize,
    #[serde(rename = "startLine")]
    pub start_line: usize,
    #[serde(rename = "totalLines")]
    pub total_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    pub file: TextFilePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredPatchHunk {
    #[serde(rename = "oldStart")]
    pub old_start: usize,
    #[serde(rename = "oldLines")]
    pub old_lines: usize,
    #[serde(rename = "newStart")]
    pub new_start: usize,
    #[serde(rename = "newLines")]
    pub new_lines: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "originalFile")]
    pub original_file: Option<String>,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditFileOutput {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "oldString")]
    pub old_string: String,
    #[serde(rename = "newString")]
    pub new_string: String,
    #[serde(rename = "originalFile")]
    pub original_file: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "userModified")]
    pub user_modified: bool,
    #[serde(rename = "replaceAll")]
    pub replace_all: bool,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobSearchOutput {
    #[serde(rename = "durationMs")]
    pub duration_ms: u128,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepSearchInput {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    #[serde(rename = "output_mode")]
    pub output_mode: Option<String>,
    #[serde(rename = "-B")]
    pub before: Option<usize>,
    #[serde(rename = "-A")]
    pub after: Option<usize>,
    #[serde(rename = "-C")]
    pub context_short: Option<usize>,
    pub context: Option<usize>,
    #[serde(rename = "-n")]
    pub line_numbers: Option<bool>,
    #[serde(rename = "-i")]
    pub case_insensitive: Option<bool>,
    #[serde(rename = "type")]
    pub file_type: Option<String>,
    pub head_limit: Option<usize>,
    pub offset: Option<usize>,
    pub multiline: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepSearchOutput {
    pub mode: Option<String>,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    pub content: Option<String>,
    #[serde(rename = "numLines")]
    pub num_lines: Option<usize>,
    #[serde(rename = "numMatches")]
    pub num_matches: Option<usize>,
    #[serde(rename = "appliedLimit")]
    pub applied_limit: Option<usize>,
    #[serde(rename = "appliedOffset")]
    pub applied_offset: Option<usize>,
}

pub fn read_file(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> io::Result<ReadFileOutput> {
    let absolute_path = normalize_path(path)?;

    // Check file size before reading
    let metadata = fs::metadata(&absolute_path)?;
    if metadata.len() > MAX_READ_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "file is too large ({} bytes, max {} bytes)",
                metadata.len(),
                MAX_READ_SIZE
            ),
        ));
    }

    // Detect binary files
    if is_binary_file(&absolute_path)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file appears to be binary",
        ));
    }

    let content = fs::read_to_string(&absolute_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let requested_offset = offset.unwrap_or(1);
    let start_index = if requested_offset == 0 {
        0
    } else {
        requested_offset.saturating_sub(1)
    };
    let clamped_start_index = start_index.min(lines.len());
    let end_index = limit.map_or(lines.len(), |limit| {
        clamped_start_index.saturating_add(limit).min(lines.len())
    });
    let selected = lines[clamped_start_index..end_index].join("\n");

    Ok(ReadFileOutput {
        kind: String::from("text"),
        file: TextFilePayload {
            file_path: absolute_path.to_string_lossy().into_owned(),
            content: selected,
            num_lines: end_index.saturating_sub(clamped_start_index),
            start_line: requested_offset,
            total_lines: lines.len(),
        },
    })
}

pub fn write_file(path: &str, content: &str) -> io::Result<WriteFileOutput> {
    if content.len() > MAX_WRITE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "content is too large ({} bytes, max {} bytes)",
                content.len(),
                MAX_WRITE_SIZE
            ),
        ));
    }

    let absolute_path = normalize_path_allow_missing(path)?;
    let original_file = fs::read_to_string(&absolute_path).ok();
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&absolute_path, content)?;

    Ok(WriteFileOutput {
        kind: if original_file.is_some() {
            String::from("update")
        } else {
            String::from("create")
        },
        file_path: absolute_path.to_string_lossy().into_owned(),
        content: content.to_owned(),
        structured_patch: make_patch(original_file.as_deref().unwrap_or(""), content),
        original_file,
        git_diff: None,
    })
}

pub fn edit_file(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> io::Result<EditFileOutput> {
    let absolute_path = normalize_path_allow_missing(path)?;
    let original_file = fs::read_to_string(&absolute_path).or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound && old_string.is_empty() {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    if old_string == new_string {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_string and new_string must differ",
        ));
    }
    if !old_string.is_empty() && !original_file.contains(old_string) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "old_string not found in file",
        ));
    }

    let updated = if replace_all {
        original_file.replace(old_string, new_string)
    } else {
        original_file.replacen(old_string, new_string, 1)
    };
    fs::write(&absolute_path, &updated)?;

    Ok(EditFileOutput {
        file_path: absolute_path.to_string_lossy().into_owned(),
        old_string: old_string.to_owned(),
        new_string: new_string.to_owned(),
        original_file: original_file.clone(),
        structured_patch: make_patch(&original_file, &updated),
        user_modified: false,
        replace_all,
        git_diff: None,
    })
}

pub fn glob_search(pattern: &str, path: Option<&str>) -> io::Result<GlobSearchOutput> {
    let started = Instant::now();
    let cwd = std::env::current_dir()?;
    let base_dir = path
        .map(normalize_path)
        .transpose()?
        .unwrap_or_else(|| cwd.clone());
    let search_pattern = if Path::new(pattern).is_absolute() {
        pattern.to_owned()
    } else {
        base_dir.join(pattern).to_string_lossy().into_owned()
    };

    let mut matches = Vec::new();
    let entries = glob::glob(&search_pattern)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    for entry in entries.flatten() {
        if entry.is_file() {
            matches.push(entry);
        }
    }

    matches.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .map(Reverse)
    });

    let truncated = matches.len() > 100;
    let filenames = matches
        .into_iter()
        .take(100)
        .map(|path| relative_display_path(&path, &cwd))
        .collect::<Vec<_>>();

    Ok(GlobSearchOutput {
        duration_ms: started.elapsed().as_millis(),
        num_files: filenames.len(),
        filenames,
        truncated,
    })
}

pub fn grep_search(input: &GrepSearchInput) -> io::Result<GrepSearchOutput> {
    let cwd = std::env::current_dir()?;
    let base_path = input
        .path
        .as_deref()
        .map(normalize_path)
        .transpose()?
        .unwrap_or_else(|| cwd.clone());

    let regex = build_grep_regex(input)?;
    let glob_filter = build_grep_glob_filter(input)?;
    let file_type = input.file_type.as_deref();
    let output_mode = input
        .output_mode
        .clone()
        .unwrap_or_else(|| String::from("files_with_matches"));
    let context = input.context.or(input.context_short).unwrap_or(0);

    let mut matched_files = Vec::new();
    let mut content_lines = Vec::new();
    let mut count_entries = Vec::new();

    for file_path in collect_search_files(&base_path)? {
        if !matches_optional_filters(&file_path, glob_filter.as_ref(), file_type) {
            continue;
        }

        let Ok(file_contents) = fs::read_to_string(&file_path) else {
            continue;
        };

        if output_mode == "count" {
            let count = regex.find_iter(&file_contents).count();
            if count > 0 {
                count_entries.push((file_path, count));
            }
            continue;
        }

        let lines: Vec<&str> = file_contents.lines().collect();
        let matched_lines = matching_line_indices(&lines, &regex);

        if matched_lines.is_empty() {
            continue;
        }

        matched_files.push(file_path.clone());
        if output_mode == "content" {
            append_content_matches(
                &mut content_lines,
                &file_path,
                &cwd,
                &lines,
                &matched_lines,
                input,
                context,
            );
        }
    }

    if output_mode == "content" {
        return Ok(build_grep_content_output(content_lines, input, output_mode));
    }

    if output_mode == "count" {
        return Ok(build_grep_count_output(
            count_entries,
            &cwd,
            input,
            output_mode,
        ));
    }

    Ok(build_grep_files_output(
        matched_files,
        &cwd,
        input,
        output_mode,
    ))
}

fn collect_search_files(base_path: &Path) -> io::Result<Vec<PathBuf>> {
    if base_path.is_file() {
        return Ok(vec![base_path.to_path_buf()]);
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(base_path) {
        let entry = entry.map_err(|error| io::Error::other(error.to_string()))?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    Ok(files)
}

fn matches_optional_filters(
    path: &Path,
    glob_filter: Option<&Pattern>,
    file_type: Option<&str>,
) -> bool {
    if let Some(glob_filter) = glob_filter {
        let path_string = path.to_string_lossy();
        if !glob_filter.matches(&path_string) && !glob_filter.matches_path(path) {
            return false;
        }
    }

    if let Some(file_type) = file_type {
        let extension = path.extension().and_then(|extension| extension.to_str());
        if extension != Some(file_type) {
            return false;
        }
    }

    true
}

fn build_grep_regex(input: &GrepSearchInput) -> io::Result<Regex> {
    RegexBuilder::new(&input.pattern)
        .case_insensitive(input.case_insensitive.unwrap_or(false))
        .dot_matches_new_line(input.multiline.unwrap_or(false))
        .build()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))
}

fn build_grep_glob_filter(input: &GrepSearchInput) -> io::Result<Option<Pattern>> {
    input
        .glob
        .as_deref()
        .map(Pattern::new)
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))
}

fn matching_line_indices(lines: &[&str], regex: &Regex) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| regex.is_match(line).then_some(index))
        .collect()
}

fn append_content_matches(
    content_lines: &mut Vec<String>,
    file_path: &Path,
    cwd: &Path,
    lines: &[&str],
    matched_lines: &[usize],
    input: &GrepSearchInput,
    context: usize,
) {
    let display_path = relative_display_path(file_path, cwd);
    for index in matched_lines {
        let start = index.saturating_sub(input.before.unwrap_or(context));
        let end = (index + input.after.unwrap_or(context) + 1).min(lines.len());
        for (current, line) in lines.iter().enumerate().take(end).skip(start) {
            let prefix = if input.line_numbers.unwrap_or(true) {
                format!("{display_path}:{}:", current + 1)
            } else {
                format!("{display_path}:")
            };
            content_lines.push(format!("{prefix}{line}"));
        }
    }
}

fn build_grep_content_output(
    content_lines: Vec<String>,
    input: &GrepSearchInput,
    output_mode: String,
) -> GrepSearchOutput {
    let (lines, applied_limit, applied_offset) =
        apply_limit(content_lines, input.head_limit, input.offset);
    GrepSearchOutput {
        mode: Some(output_mode),
        num_files: 0,
        filenames: Vec::new(),
        num_lines: Some(lines.len()),
        content: Some(lines.join("\n")),
        num_matches: None,
        applied_limit,
        applied_offset,
    }
}

fn build_grep_count_output(
    count_entries: Vec<(PathBuf, usize)>,
    cwd: &Path,
    input: &GrepSearchInput,
    output_mode: String,
) -> GrepSearchOutput {
    let count_lines = count_entries
        .into_iter()
        .map(|(path, count)| format!("{}:{count}", relative_display_path(&path, cwd)))
        .collect::<Vec<_>>();
    let (lines, applied_limit, applied_offset) =
        apply_limit(count_lines, input.head_limit, input.offset);
    let total_matches = lines
        .iter()
        .filter_map(|line| line.rsplit_once(':'))
        .filter_map(|(_, count)| count.parse::<usize>().ok())
        .sum::<usize>();

    GrepSearchOutput {
        mode: Some(output_mode),
        num_files: lines.len(),
        filenames: Vec::new(),
        content: Some(lines.join("\n")),
        num_lines: None,
        num_matches: Some(total_matches),
        applied_limit,
        applied_offset,
    }
}

fn build_grep_files_output(
    mut matched_files: Vec<PathBuf>,
    cwd: &Path,
    input: &GrepSearchInput,
    output_mode: String,
) -> GrepSearchOutput {
    sort_paths_by_modified_desc(&mut matched_files);
    let relative_matches = matched_files
        .into_iter()
        .map(|path| relative_display_path(&path, cwd))
        .collect::<Vec<_>>();
    let (filenames, applied_limit, applied_offset) =
        apply_limit(relative_matches, input.head_limit, input.offset);

    GrepSearchOutput {
        mode: Some(output_mode),
        num_files: filenames.len(),
        filenames,
        content: None,
        num_lines: None,
        num_matches: None,
        applied_limit,
        applied_offset,
    }
}

fn apply_limit<T>(
    items: Vec<T>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> (Vec<T>, Option<usize>, Option<usize>) {
    let offset_value = offset.unwrap_or(0);
    let mut items = items.into_iter().skip(offset_value).collect::<Vec<_>>();
    let explicit_limit = limit.unwrap_or(250);
    if explicit_limit == 0 {
        return (items, None, (offset_value > 0).then_some(offset_value));
    }

    let truncated = items.len() > explicit_limit;
    items.truncate(explicit_limit);
    (
        items,
        truncated.then_some(explicit_limit),
        (offset_value > 0).then_some(offset_value),
    )
}

fn sort_paths_by_modified_desc(paths: &mut [PathBuf]) {
    paths.sort_by(|left, right| {
        let left_modified = fs::metadata(left)
            .and_then(|metadata| metadata.modified())
            .ok();
        let right_modified = fs::metadata(right)
            .and_then(|metadata| metadata.modified())
            .ok();
        right_modified.cmp(&left_modified).then_with(|| {
            let left_display = left.to_string_lossy();
            let right_display = right.to_string_lossy();
            left_display.as_ref().cmp(right_display.as_ref())
        })
    });
}

fn relative_display_path(path: &Path, cwd: &Path) -> String {
    let normalized_path = normalize_display_path(path);
    let normalized_cwd = normalize_display_path(cwd);
    diff_paths(&normalized_path, &normalized_cwd)
        .unwrap_or(normalized_path)
        .to_string_lossy()
        .into_owned()
}

fn normalize_display_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(stripped) = raw.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{stripped}"));
        }
        if let Some(stripped) = raw.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path.to_path_buf()
}

fn diff_paths(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let path_components = path.components().collect::<Vec<_>>();
    let base_components = base.components().collect::<Vec<_>>();

    match (path_components.first(), base_components.first()) {
        (Some(Component::Prefix(path_prefix)), Some(Component::Prefix(base_prefix)))
            if path_prefix.kind() != base_prefix.kind() =>
        {
            return None;
        }
        (Some(Component::RootDir), Some(Component::Prefix(_)))
        | (Some(Component::Prefix(_)), Some(Component::RootDir)) => return None,
        _ => {}
    }

    let common_prefix_len = path_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative = PathBuf::new();
    for _ in common_prefix_len..base_components.len() {
        relative.push("..");
    }
    for component in &path_components[common_prefix_len..] {
        relative.push(component.as_os_str());
    }

    if relative.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(relative)
    }
}

fn make_patch(original: &str, updated: &str) -> Vec<StructuredPatchHunk> {
    let mut lines = Vec::new();
    for line in original.lines() {
        lines.push(format!("-{line}"));
    }
    for line in updated.lines() {
        lines.push(format!("+{line}"));
    }

    vec![StructuredPatchHunk {
        old_start: 1,
        old_lines: original.lines().count(),
        new_start: 1,
        new_lines: updated.lines().count(),
        lines,
    }]
}

fn absolute_candidate(path: &str) -> io::Result<PathBuf> {
    if Path::new(path).is_absolute() {
        Ok(PathBuf::from(path))
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn canonicalize_with_fallback<F>(candidate: PathBuf, canonicalize: F) -> io::Result<PathBuf>
where
    F: FnOnce(&Path) -> io::Result<PathBuf>,
{
    match canonicalize(&candidate) {
        Ok(canonical) => Ok(clean_path_buf(canonical)),
        Err(error) => {
            if candidate.exists() {
                Ok(clean_path_buf(candidate))
            } else {
                Err(error)
            }
        }
    }
}

fn normalize_path(path: &str) -> io::Result<PathBuf> {
    canonicalize_with_fallback(absolute_candidate(path)?, Path::canonicalize)
}

fn normalize_path_allow_missing(path: &str) -> io::Result<PathBuf> {
    let candidate = absolute_candidate(path)?;

    if let Ok(canonical) = candidate.canonicalize() {
        return Ok(clean_path_buf(canonical));
    }

    if let Some(parent) = candidate.parent() {
        let canonical_parent = parent
            .canonicalize()
            .map_or_else(|_| clean_path_buf(parent.to_path_buf()), clean_path_buf);
        if let Some(name) = candidate.file_name() {
            return Ok(clean_path_buf(canonical_parent.join(name)));
        }
    }

    Ok(clean_path_buf(candidate))
}

pub fn resolve_tool_path(path: &str) -> io::Result<PathBuf> {
    normalize_path(path)
}

pub fn resolve_tool_path_allow_missing(path: &str) -> io::Result<PathBuf> {
    normalize_path_allow_missing(path)
}

fn clean_path_buf(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(stripped) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path
}

/// Read a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn read_file_in_workspace(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    workspace_root: &Path,
) -> io::Result<ReadFileOutput> {
    let absolute_path = normalize_path(path)?;
    let canonical_root = workspace_root.canonicalize().map_or_else(
        |_| clean_path_buf(workspace_root.to_path_buf()),
        clean_path_buf,
    );
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    read_file(path, offset, limit)
}

/// Write a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn write_file_in_workspace(
    path: &str,
    content: &str,
    workspace_root: &Path,
) -> io::Result<WriteFileOutput> {
    let absolute_path = normalize_path_allow_missing(path)?;
    let canonical_root = workspace_root.canonicalize().map_or_else(
        |_| clean_path_buf(workspace_root.to_path_buf()),
        clean_path_buf,
    );
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    write_file(path, content)
}

/// Edit a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn edit_file_in_workspace(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    workspace_root: &Path,
) -> io::Result<EditFileOutput> {
    let absolute_path = normalize_path(path)?;
    let canonical_root = workspace_root.canonicalize().map_or_else(
        |_| clean_path_buf(workspace_root.to_path_buf()),
        clean_path_buf,
    );
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    edit_file(path, old_string, new_string, replace_all)
}

/// Check whether a path is a symlink that resolves outside the workspace.
#[allow(dead_code)]
pub fn is_symlink_escape(path: &Path, workspace_root: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_symlink() {
        return Ok(false);
    }
    let resolved = clean_path_buf(path.canonicalize()?);
    let canonical_root = workspace_root.canonicalize().map_or_else(
        |_| clean_path_buf(workspace_root.to_path_buf()),
        clean_path_buf,
    );
    Ok(!resolved.starts_with(&canonical_root))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        canonicalize_with_fallback, edit_file, glob_search, grep_search, is_symlink_escape,
        read_file, read_file_in_workspace, write_file, GrepSearchInput, MAX_WRITE_SIZE,
    };

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("clawd-native-{name}-{unique}"))
    }

    #[test]
    fn reads_and_writes_files() {
        let path = temp_path("read-write.txt");
        let write_output = write_file(path.to_string_lossy().as_ref(), "one\ntwo\nthree")
            .expect("write should succeed");
        assert_eq!(write_output.kind, "create");

        let read_output = read_file(path.to_string_lossy().as_ref(), Some(1), Some(1))
            .expect("read should succeed");
        assert_eq!(read_output.file.content, "one");
        assert_eq!(read_output.file.start_line, 1);
    }

    #[test]
    fn edits_file_contents() {
        let path = temp_path("edit.txt");
        write_file(path.to_string_lossy().as_ref(), "alpha beta alpha")
            .expect("initial write should succeed");
        let output = edit_file(path.to_string_lossy().as_ref(), "alpha", "omega", true)
            .expect("edit should succeed");
        assert!(output.replace_all);
    }

    #[test]
    fn rejects_binary_files() {
        let path = temp_path("binary-test.bin");
        std::fs::write(&path, b"\x00\x01\x02\x03binary content").expect("write should succeed");
        let result = read_file(path.to_string_lossy().as_ref(), None, None);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("binary"));
    }

    #[test]
    fn rejects_oversized_writes() {
        let path = temp_path("oversize-write.txt");
        let huge = "x".repeat(MAX_WRITE_SIZE + 1);
        let result = write_file(path.to_string_lossy().as_ref(), &huge);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn falls_back_to_existing_absolute_path_when_canonicalize_fails() {
        let path = temp_path("canonicalize-fallback.txt");
        write_file(path.to_string_lossy().as_ref(), "hello").expect("write should succeed");

        let resolved =
            canonicalize_with_fallback(path.clone(), |_| Err(io::Error::from_raw_os_error(1005)))
                .expect("existing path should fall back to candidate");

        assert_eq!(resolved, path);
    }

    #[test]
    fn preserves_not_found_when_canonicalize_fails_for_missing_path() {
        let path = temp_path("missing-canonicalize-fallback.txt");

        let error = canonicalize_with_fallback(path, |_| Err(io::Error::from_raw_os_error(1005)))
            .expect_err("missing path should still error");

        assert_eq!(error.raw_os_error(), Some(1005));
    }

    #[test]
    fn enforces_workspace_boundary() {
        let workspace = temp_path("workspace-boundary");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be created");
        let inside = workspace.join("inside.txt");
        write_file(inside.to_string_lossy().as_ref(), "safe content")
            .expect("write inside workspace should succeed");

        // Reading inside workspace should succeed
        let result =
            read_file_in_workspace(inside.to_string_lossy().as_ref(), None, None, &workspace);
        assert!(result.is_ok());

        // Reading outside workspace should fail
        let outside = temp_path("outside-boundary.txt");
        write_file(outside.to_string_lossy().as_ref(), "unsafe content")
            .expect("write outside should succeed");
        let result =
            read_file_in_workspace(outside.to_string_lossy().as_ref(), None, None, &workspace);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("escapes workspace"));
    }

    #[test]
    fn detects_symlink_escape() {
        let workspace = temp_path("symlink-workspace");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be created");
        let outside = temp_path("symlink-target.txt");
        std::fs::write(&outside, "target content").expect("target should write");

        #[cfg(unix)]
        let link_path = workspace.join("escape-link.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, &link_path).expect("symlink should create");
            assert!(is_symlink_escape(&link_path, &workspace).expect("check should succeed"));
        }

        // Non-symlink file should not be an escape
        let normal = workspace.join("normal.txt");
        std::fs::write(&normal, "normal content").expect("normal file should write");
        assert!(!is_symlink_escape(&normal, &workspace).expect("check should succeed"));
    }

    #[test]
    fn globs_and_greps_directory() {
        let dir = temp_path("search-dir");
        std::fs::create_dir_all(&dir).expect("directory should be created");
        let file = dir.join("demo.rs");
        write_file(
            file.to_string_lossy().as_ref(),
            "fn main() {\n println!(\"hello\");\n}\n",
        )
        .expect("file write should succeed");

        let globbed = glob_search("**/*.rs", Some(dir.to_string_lossy().as_ref()))
            .expect("glob should succeed");
        assert_eq!(globbed.num_files, 1);

        let grep_output = grep_search(&GrepSearchInput {
            pattern: String::from("hello"),
            path: Some(dir.to_string_lossy().into_owned()),
            glob: Some(String::from("**/*.rs")),
            output_mode: Some(String::from("content")),
            before: None,
            after: None,
            context_short: None,
            context: None,
            line_numbers: Some(true),
            case_insensitive: Some(false),
            file_type: None,
            head_limit: Some(10),
            offset: Some(0),
            multiline: Some(false),
        })
        .expect("grep should succeed");
        assert!(grep_output.content.unwrap_or_default().contains("hello"));
    }
}
