use crate::prompt::SYSTEM_PROMPT_DYNAMIC_BOUNDARY;
use crate::session::{ContentBlock, MessageRole, Session};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCategoryUsage {
    pub name: String,
    pub tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSectionUsage {
    pub name: String,
    pub tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextUsageReport {
    pub total_tokens: usize,
    pub system_prompt_tokens: usize,
    pub message_tokens: usize,
    pub message_count: usize,
    pub categories: Vec<ContextCategoryUsage>,
    pub system_prompt_sections: Vec<ContextSectionUsage>,
}

#[must_use]
pub fn analyze_context_usage(system_prompt: &[String], session: &Session) -> ContextUsageReport {
    let system_prompt_sections = system_prompt
        .iter()
        .enumerate()
        .map(|(index, section)| ContextSectionUsage {
            name: describe_system_prompt_section(index, section),
            tokens: estimate_text_tokens(section),
        })
        .collect::<Vec<_>>();
    let system_prompt_tokens = system_prompt_sections
        .iter()
        .map(|section| section.tokens)
        .sum();

    let mut system_message_tokens = 0;
    let mut user_message_tokens = 0;
    let mut assistant_message_tokens = 0;
    let mut tool_call_tokens = 0;
    let mut tool_result_tokens = 0;

    for message in &session.messages {
        for block in &message.blocks {
            match block {
                ContentBlock::Text { text } => {
                    let tokens = estimate_text_tokens(text);
                    match message.role {
                        MessageRole::System => system_message_tokens += tokens,
                        MessageRole::User => user_message_tokens += tokens,
                        MessageRole::Assistant => assistant_message_tokens += tokens,
                        MessageRole::Tool => tool_result_tokens += tokens,
                    }
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    tool_call_tokens += estimate_text_tokens(name) + estimate_text_tokens(input);
                }
                ContentBlock::ToolResult {
                    tool_name,
                    output,
                    is_error,
                    ..
                } => {
                    tool_result_tokens += estimate_text_tokens(tool_name)
                        + estimate_text_tokens(output)
                        + usize::from(*is_error);
                }
            }
        }
    }

    let categories = vec![
        ContextCategoryUsage {
            name: "System prompt".to_string(),
            tokens: system_prompt_tokens,
        },
        ContextCategoryUsage {
            name: "System messages".to_string(),
            tokens: system_message_tokens,
        },
        ContextCategoryUsage {
            name: "User messages".to_string(),
            tokens: user_message_tokens,
        },
        ContextCategoryUsage {
            name: "Assistant messages".to_string(),
            tokens: assistant_message_tokens,
        },
        ContextCategoryUsage {
            name: "Tool calls".to_string(),
            tokens: tool_call_tokens,
        },
        ContextCategoryUsage {
            name: "Tool results".to_string(),
            tokens: tool_result_tokens,
        },
    ];
    let message_tokens = categories
        .iter()
        .filter(|category| category.name != "System prompt")
        .map(|category| category.tokens)
        .sum();

    ContextUsageReport {
        total_tokens: system_prompt_tokens + message_tokens,
        system_prompt_tokens,
        message_tokens,
        message_count: session.messages.len(),
        categories,
        system_prompt_sections,
    }
}

fn describe_system_prompt_section(index: usize, section: &str) -> String {
    let trimmed = section.trim();
    if trimmed.is_empty() {
        return format!("Section {}", index + 1);
    }
    if trimmed == SYSTEM_PROMPT_DYNAMIC_BOUNDARY {
        return "Dynamic boundary".to_string();
    }
    let Some(first_line) = trimmed.lines().find(|line| !line.trim().is_empty()) else {
        return format!("Section {}", index + 1);
    };
    if let Some(heading) = first_line.trim().strip_prefix('#') {
        let heading = heading.trim_start_matches('#').trim();
        if !heading.is_empty() {
            return heading.to_string();
        }
    }
    if index == 0 {
        return "Intro".to_string();
    }
    truncate_label(first_line.trim(), 48)
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push('…');
    output
}

fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        chars / 4 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::analyze_context_usage;
    use crate::session::{ContentBlock, ConversationMessage, MessageRole, Session};

    #[test]
    fn analyzes_system_prompt_sections_and_message_categories() {
        let report = analyze_context_usage(
            &[
                "You are a coding assistant.".to_string(),
                "# System\nUse tools carefully.".to_string(),
                "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__".to_string(),
                "# Project context\nWorking directory: repo".to_string(),
            ],
            &Session {
                version: 1,
                messages: vec![
                    ConversationMessage {
                        role: MessageRole::System,
                        blocks: vec![ContentBlock::Text {
                            text: "Summary:\nEarlier context".to_string(),
                        }],
                        usage: None,
                    },
                    ConversationMessage::user_text("Please inspect src/main.rs"),
                    ConversationMessage::assistant(vec![
                        ContentBlock::Text {
                            text: "I will inspect it.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "tool-1".to_string(),
                            name: "Read".to_string(),
                            input: "{\"file_path\":\"src/main.rs\"}".to_string(),
                        },
                    ]),
                    ConversationMessage::tool_result("tool-1", "Read", "fn main() {}", false),
                ],
            },
        );

        assert_eq!(report.message_count, 4);
        assert_eq!(report.system_prompt_sections[0].name, "Intro");
        assert_eq!(report.system_prompt_sections[1].name, "System");
        assert_eq!(report.system_prompt_sections[2].name, "Dynamic boundary");
        assert_eq!(report.system_prompt_sections[3].name, "Project context");
        assert!(report.total_tokens > 0);
        assert!(report.system_prompt_tokens > 0);
        assert!(report.message_tokens > 0);
        assert!(report
            .categories
            .iter()
            .any(|category| category.name == "Tool calls" && category.tokens > 0));
        assert!(report
            .categories
            .iter()
            .any(|category| category.name == "Tool results" && category.tokens > 0));
    }
}
