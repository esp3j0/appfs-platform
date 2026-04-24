use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillExecutionContext {
    Fork,
}

impl SkillExecutionContext {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fork => "fork",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillDocument {
    pub resolved_name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub has_user_specified_description: bool,
    pub allowed_tools: Vec<String>,
    pub argument_hint: Option<String>,
    pub argument_names: Vec<String>,
    pub when_to_use: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
    pub disable_model_invocation: bool,
    pub user_invocable: bool,
    pub hooks: Option<JsonValue>,
    pub execution_context: Option<SkillExecutionContext>,
    pub agent: Option<String>,
    pub effort: Option<String>,
    pub paths: Option<Vec<String>>,
    pub shell: Option<String>,
    pub markdown_content: String,
}

impl SkillDocument {
    #[must_use]
    pub fn user_facing_name(&self) -> &str {
        self.display_name
            .as_deref()
            .unwrap_or(self.resolved_name.as_str())
    }

    #[must_use]
    pub fn render_markdown_with_arguments(&self, args: Option<&str>) -> String {
        substitute_arguments(&self.markdown_content, args, true, &self.argument_names)
    }
}

pub fn load_skill_document(
    path: &Path,
    description_fallback_label: &str,
) -> io::Result<SkillDocument> {
    let contents = fs::read_to_string(path)?;
    let resolved_name = default_skill_name(path);
    Ok(parse_skill_document(
        &contents,
        resolved_name,
        description_fallback_label,
    ))
}

#[must_use]
pub fn parse_skill_document(
    contents: &str,
    resolved_name: String,
    description_fallback_label: &str,
) -> SkillDocument {
    let (frontmatter, markdown_content) = split_frontmatter(contents);
    let frontmatter = frontmatter.and_then(parse_frontmatter_mapping).or_else(|| {
        frontmatter.and_then(|text| parse_frontmatter_mapping(&quote_problematic_values(text)))
    });

    let display_name = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "name"));
    let user_specified_description = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_description(mapping, "description"));
    let description = user_specified_description.clone().unwrap_or_else(|| {
        extract_description_from_markdown(markdown_content, description_fallback_label)
    });

    let allowed_tools = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "allowed-tools"))
        .map(parse_allowed_tools)
        .unwrap_or_default();
    let argument_hint = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "argument-hint"));
    let argument_names = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "arguments"))
        .map(parse_argument_names)
        .unwrap_or_default();
    let when_to_use = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "when_to_use"));
    let version = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "version"));
    let model = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "model"))
        .filter(|value| !value.eq_ignore_ascii_case("inherit"));
    let disable_model_invocation = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "disable-model-invocation"))
        .is_some_and(parse_boolean_frontmatter);
    let user_invocable = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "user-invocable"))
        .is_none_or(parse_boolean_frontmatter);
    let hooks = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "hooks"))
        .and_then(yaml_to_json);
    let execution_context = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "context"))
        .and_then(|value| {
            if value.eq_ignore_ascii_case("fork") {
                Some(SkillExecutionContext::Fork)
            } else {
                None
            }
        });
    let agent = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "agent"));
    let effort = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_scalar_string(mapping, "effort"));
    let paths = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_value(mapping, "paths"))
        .and_then(parse_paths);
    let shell = frontmatter
        .as_ref()
        .and_then(|mapping| mapping_string(mapping, "shell"))
        .and_then(|value| parse_shell(&value));

    SkillDocument {
        resolved_name,
        display_name,
        description,
        has_user_specified_description: user_specified_description.is_some(),
        allowed_tools,
        argument_hint,
        argument_names,
        when_to_use,
        version,
        model,
        disable_model_invocation,
        user_invocable,
        hooks,
        execution_context,
        agent,
        effort,
        paths,
        shell,
        markdown_content: markdown_content.to_string(),
    }
}

#[must_use]
pub fn extract_skill_frontmatter_name(contents: &str) -> Option<String> {
    let (frontmatter, _) = split_frontmatter(contents);
    frontmatter
        .and_then(parse_frontmatter_mapping)
        .or_else(|| {
            frontmatter.and_then(|text| parse_frontmatter_mapping(&quote_problematic_values(text)))
        })
        .and_then(|mapping| mapping_string(&mapping, "name"))
}

fn default_skill_name(path: &Path) -> String {
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("SKILL.md"))
    {
        return path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_string())
            .or_else(|| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "skill".to_string());
    }

    path.file_stem().map_or_else(
        || "skill".to_string(),
        |stem| stem.to_string_lossy().to_string(),
    )
}

fn split_frontmatter(contents: &str) -> (Option<&str>, &str) {
    let mut lines = contents.split_inclusive('\n');
    let Some(first_line) = lines.next() else {
        return (None, contents);
    };
    if first_line.trim() != "---" {
        return (None, contents);
    }

    let mut cursor = first_line.len();
    for line in lines {
        if line.trim() == "---" {
            let frontmatter = &contents[first_line.len()..cursor];
            let body = &contents[cursor + line.len()..];
            return (Some(frontmatter), body);
        }
        cursor += line.len();
    }

    (None, contents)
}

fn parse_frontmatter_mapping(frontmatter: &str) -> Option<Mapping> {
    match serde_yaml::from_str::<YamlValue>(frontmatter).ok()? {
        YamlValue::Mapping(mapping) => Some(mapping),
        _ => None,
    }
}

fn quote_problematic_values(frontmatter: &str) -> String {
    frontmatter
        .lines()
        .map(|line| {
            if line.starts_with(' ') || line.starts_with('\t') || line.starts_with('-') {
                return line.to_string();
            }

            let Some((key, value)) = line.split_once(':') else {
                return line.to_string();
            };
            if !key
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '-'))
            {
                return line.to_string();
            }
            let Some(value) = value.strip_prefix(' ') else {
                return line.to_string();
            };
            if value.is_empty() || is_quoted_yaml(value) || !has_yaml_special_chars(value) {
                return line.to_string();
            }

            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            format!("{key}: \"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_quoted_yaml(value: &str) -> bool {
    (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
}

fn has_yaml_special_chars(value: &str) -> bool {
    value.contains(": ")
        || value.chars().any(|ch| {
            matches!(
                ch,
                '{' | '}' | '[' | ']' | '*' | '&' | '#' | '!' | '|' | '>' | '%' | '@' | '`'
            )
        })
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn mapping_string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping_value(mapping, key).and_then(yaml_stringish)
}

fn mapping_scalar_string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping_value(mapping, key).and_then(yaml_scalar_string)
}

fn mapping_description(mapping: &Mapping, key: &str) -> Option<String> {
    mapping_value(mapping, key).and_then(yaml_description_string)
}

fn yaml_scalar_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::Bool(value) => Some(value.to_string()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

fn yaml_stringish(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::Sequence(items) => {
            let values = items
                .iter()
                .filter_map(yaml_scalar_string)
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| values.join(","))
        }
        _ => yaml_scalar_string(value),
    }
}

fn yaml_description_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::Bool(value) => Some(value.to_string()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

fn parse_boolean_frontmatter(value: &YamlValue) -> bool {
    matches!(value, YamlValue::Bool(true))
        || matches!(value, YamlValue::String(text) if text == "true")
}

fn parse_allowed_tools(value: &YamlValue) -> Vec<String> {
    let segments = match value {
        YamlValue::String(text) => vec![text.clone()],
        YamlValue::Sequence(items) => items
            .iter()
            .filter_map(yaml_scalar_string)
            .collect::<Vec<_>>(),
        _ => return Vec::new(),
    };

    parse_tool_list_from_cli(&segments)
}

fn parse_tool_list_from_cli(values: &[String]) -> Vec<String> {
    let mut parsed = Vec::new();

    for value in values {
        if value.trim().is_empty() {
            continue;
        }

        let mut current = String::new();
        let mut depth = 0_u32;
        for ch in value.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    current.push(ch);
                }
                ',' | ' ' if depth == 0 => {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        parsed.push(trimmed.to_string());
                    }
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        let trimmed = current.trim();
        if !trimmed.is_empty() {
            parsed.push(trimmed.to_string());
        }
    }

    parsed
}

fn parse_argument_names(value: &YamlValue) -> Vec<String> {
    let is_valid =
        |name: &str| !name.trim().is_empty() && !name.chars().all(|ch| ch.is_ascii_digit());
    match value {
        YamlValue::String(text) => text
            .split_whitespace()
            .filter(|name| is_valid(name))
            .map(ToString::to_string)
            .collect(),
        YamlValue::Sequence(items) => items
            .iter()
            .filter_map(yaml_scalar_string)
            .filter(|name| is_valid(name))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_paths(value: &YamlValue) -> Option<Vec<String>> {
    let inputs = match value {
        YamlValue::String(text) => vec![text.clone()],
        YamlValue::Sequence(items) => items
            .iter()
            .filter_map(yaml_scalar_string)
            .collect::<Vec<_>>(),
        _ => return None,
    };

    let patterns = inputs
        .iter()
        .flat_map(|text| split_path_in_frontmatter(text))
        .map(|pattern| pattern.strip_suffix("/**").unwrap_or(&pattern).to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();

    if patterns.is_empty() || patterns.iter().all(|pattern| pattern == "**") {
        None
    } else {
        Some(patterns)
    }
}

fn split_path_in_frontmatter(input: &str) -> Vec<String> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    let mut brace_depth = 0_i32;

    for ch in input.chars() {
        match ch {
            '{' => {
                brace_depth += 1;
                current.push(ch);
            }
            '}' => {
                brace_depth -= 1;
                current.push(ch);
            }
            ',' if brace_depth == 0 => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_string());
    }

    parts
        .into_iter()
        .flat_map(|pattern| expand_braces(&pattern))
        .collect()
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open_index) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(close_index) = pattern[open_index + 1..].find('}') else {
        return vec![pattern.to_string()];
    };
    let close_index = open_index + 1 + close_index;
    let prefix = &pattern[..open_index];
    let alternatives = &pattern[open_index + 1..close_index];
    let suffix = &pattern[close_index + 1..];

    alternatives
        .split(',')
        .flat_map(|part| expand_braces(&format!("{prefix}{}{suffix}", part.trim())))
        .collect()
}

fn parse_shell(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "bash" => Some("bash".to_string()),
        "powershell" => Some("powershell".to_string()),
        _ => None,
    }
}

fn yaml_to_json(value: &YamlValue) -> Option<JsonValue> {
    serde_json::to_value(value).ok()
}

fn substitute_arguments(
    content: &str,
    args: Option<&str>,
    append_if_no_placeholder: bool,
    argument_names: &[String],
) -> String {
    let Some(args) = args else {
        return content.to_string();
    };

    let parsed_args = parse_arguments(args);
    let original = content.to_string();
    let mut replaced = original.clone();

    for (index, name) in argument_names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        replaced = replace_named_argument_occurrences(
            &replaced,
            name,
            parsed_args.get(index).map_or("", String::as_str),
        );
    }

    replaced = replace_arguments_indexed_occurrences(&replaced, &parsed_args);
    replaced = replace_shorthand_indexed_occurrences(&replaced, &parsed_args);
    replaced = replaced.replace("$ARGUMENTS", args);

    if replaced == original && append_if_no_placeholder && !args.is_empty() {
        replaced.push_str("\n\nARGUMENTS: ");
        replaced.push_str(args);
    }

    replaced
}

fn parse_arguments(args: &str) -> Vec<String> {
    if args.trim().is_empty() {
        return Vec::new();
    }

    try_parse_arguments(args).unwrap_or_else(|| {
        args.split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    })
}

fn try_parse_arguments(args: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = args.chars().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
            }
            '\\' if !in_single_quotes => {
                current.push(chars.next()?);
            }
            ch if !in_single_quotes && !in_double_quotes && ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch if !in_single_quotes && !in_double_quotes && is_shell_operator(ch) => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if in_single_quotes || in_double_quotes {
        return None;
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Some(tokens)
}

fn is_shell_operator(ch: char) -> bool {
    matches!(ch, '|' | '&' | ';' | '(' | ')' | '<' | '>')
}

fn replace_named_argument_occurrences(content: &str, name: &str, replacement: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut cursor = 0;

    while cursor < content.len() {
        let slice = &content[cursor..];
        if let Some(rest) = slice
            .strip_prefix('$')
            .filter(|rest| rest.starts_with(name))
        {
            let after = &rest[name.len()..];
            let next = after.chars().next();
            if next != Some('[') && !next.is_some_and(is_word_char) {
                output.push_str(replacement);
                cursor += 1 + name.len();
                continue;
            }
        }

        let ch = slice
            .chars()
            .next()
            .expect("non-empty slice should have a character");
        output.push(ch);
        cursor += ch.len_utf8();
    }

    output
}

fn replace_arguments_indexed_occurrences(content: &str, parsed_args: &[String]) -> String {
    let mut output = String::with_capacity(content.len());
    let mut cursor = 0;

    while cursor < content.len() {
        let slice = &content[cursor..];
        if let Some(rest) = slice.strip_prefix("$ARGUMENTS[") {
            let digits_len = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits_len > 0 && rest[digits_len..].starts_with(']') {
                let index = rest[..digits_len].parse::<usize>().ok();
                output.push_str(
                    index
                        .and_then(|idx| parsed_args.get(idx))
                        .map_or("", String::as_str),
                );
                cursor += "$ARGUMENTS[".len() + digits_len + 1;
                continue;
            }
        }

        let ch = slice
            .chars()
            .next()
            .expect("non-empty slice should have a character");
        output.push(ch);
        cursor += ch.len_utf8();
    }

    output
}

fn replace_shorthand_indexed_occurrences(content: &str, parsed_args: &[String]) -> String {
    let mut output = String::with_capacity(content.len());
    let mut cursor = 0;

    while cursor < content.len() {
        let slice = &content[cursor..];
        if let Some(rest) = slice.strip_prefix('$') {
            let digits_len = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits_len > 0 {
                let next = rest[digits_len..].chars().next();
                if !next.is_some_and(is_word_char) {
                    let index = rest[..digits_len].parse::<usize>().ok();
                    output.push_str(
                        index
                            .and_then(|idx| parsed_args.get(idx))
                            .map_or("", String::as_str),
                    );
                    cursor += 1 + digits_len;
                    continue;
                }
            }
        }

        let ch = slice
            .chars()
            .next()
            .expect("non-empty slice should have a character");
        output.push(ch);
        cursor += ch.len_utf8();
    }

    output
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn extract_description_from_markdown(content: &str, default_description: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let text = trimmed
            .strip_prefix("# ")
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("### "))
            .or_else(|| trimmed.strip_prefix("#### "))
            .or_else(|| trimmed.strip_prefix("##### "))
            .or_else(|| trimmed.strip_prefix("###### "))
            .unwrap_or(trimmed);
        let shortened = if text.chars().count() > 100 {
            let mut collected = text.chars().take(97).collect::<String>();
            collected.push_str("...");
            collected
        } else {
            text.to_string()
        };
        return shortened;
    }

    default_description.to_string()
}

#[cfg(test)]
mod tests {
    use super::{extract_skill_frontmatter_name, parse_skill_document, SkillExecutionContext};

    #[test]
    fn parses_rich_skill_frontmatter_fields() {
        let doc = parse_skill_document(
            "---\nname: planner\ndescription: Useful planner\narguments: [topic, depth]\nargument-hint: \"[topic] [depth]\"\nallowed-tools: \"Bash, Read(Edit)\"\nmodel: claude-3-7-sonnet\nversion: 1.2.3\ndisable-model-invocation: true\nuser-invocable: false\ncontext: fork\nagent: general-purpose\neffort: high\npaths: \"src/*.{ts,tsx}, docs/**\"\nshell: powershell\nhooks:\n  PreToolUse:\n    - matcher: Bash\n      hooks:\n        - command: echo hi\n---\n# planner\n\nBody\n",
            "planner".to_string(),
            "Skill",
        );

        assert_eq!(doc.display_name.as_deref(), Some("planner"));
        assert_eq!(doc.user_facing_name(), "planner");
        assert_eq!(doc.description, "Useful planner");
        assert_eq!(doc.argument_names, vec!["topic", "depth"]);
        assert_eq!(doc.argument_hint.as_deref(), Some("[topic] [depth]"));
        assert_eq!(doc.allowed_tools, vec!["Bash", "Read(Edit)"]);
        assert_eq!(doc.model.as_deref(), Some("claude-3-7-sonnet"));
        assert!(doc.disable_model_invocation);
        assert!(!doc.user_invocable);
        assert_eq!(doc.execution_context, Some(SkillExecutionContext::Fork));
        assert_eq!(doc.agent.as_deref(), Some("general-purpose"));
        assert_eq!(doc.effort.as_deref(), Some("high"));
        assert_eq!(
            doc.paths,
            Some(vec![
                "src/*.ts".to_string(),
                "src/*.tsx".to_string(),
                "docs".to_string(),
            ])
        );
        assert_eq!(doc.shell.as_deref(), Some("powershell"));
        assert!(doc.hooks.is_some());
        assert_eq!(doc.markdown_content, "# planner\n\nBody\n");
    }

    #[test]
    fn falls_back_to_heading_for_description_when_frontmatter_is_missing() {
        let doc = parse_skill_document(
            "# Review PR\n\nFollow the checklist.\n",
            "review-pr".to_string(),
            "Skill",
        );
        assert_eq!(doc.description, "Review PR");
        assert!(!doc.has_user_specified_description);
    }

    #[test]
    fn extracts_frontmatter_name_with_yaml_quoting_retry() {
        let name = extract_skill_frontmatter_name(
            "---\nname: \"trace\"\npaths: src/*.{ts,tsx}\n---\n# trace\n",
        );
        assert_eq!(name.as_deref(), Some("trace"));
    }

    #[test]
    fn substitutes_named_indexed_and_full_arguments() {
        let doc = parse_skill_document(
            "---\narguments: [topic, depth]\n---\nTopic: $topic\nFirst: $0\nSecond: $ARGUMENTS[1]\nRaw: $ARGUMENTS\n",
            "planner".to_string(),
            "Skill",
        );

        let rendered = doc.render_markdown_with_arguments(Some("alpha \"two words\" tail"));
        assert_eq!(
            rendered,
            "Topic: alpha\nFirst: alpha\nSecond: two words\nRaw: alpha \"two words\" tail\n"
        );
    }

    #[test]
    fn appends_arguments_block_when_no_placeholder_exists() {
        let doc = parse_skill_document("# help\n\nGuide body\n", "help".to_string(), "Skill");
        let rendered = doc.render_markdown_with_arguments(Some("overview"));
        assert_eq!(rendered, "# help\n\nGuide body\n\n\nARGUMENTS: overview");
    }

    #[test]
    fn malformed_quotes_fall_back_to_whitespace_argument_split() {
        let doc = parse_skill_document(
            "---\narguments: [first, second]\n---\nA=$first B=$second\n",
            "help".to_string(),
            "Skill",
        );
        let rendered = doc.render_markdown_with_arguments(Some("alpha \"two words"));
        assert_eq!(rendered, "A=alpha B=\"two\n");
    }
}
