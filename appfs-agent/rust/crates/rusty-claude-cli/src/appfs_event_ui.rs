use runtime::{InputEnvelope, EventTemplateTarget, render_event_template_for_target};
use serde_json::Value;

pub fn render_appfs_event_card(envelope: &InputEnvelope) -> Option<String> {
    let lines = appfs_event_card_lines(envelope);
    if lines.is_empty() {
        return None;
    }
    let title = "AppFS Wake";
    let border = "─".repeat(title.len() + 10);
    let body = lines
        .into_iter()
        .map(|line| format!("\x1b[38;5;245m│\x1b[0m {line}"))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        "\x1b[38;5;245m╭─ \x1b[1;35m{title}\x1b[0;38;5;245m ─╮\x1b[0m\n{body}\n\x1b[38;5;245m╰{border}╯\x1b[0m"
    ))
}

pub fn appfs_event_card_lines(envelope: &InputEnvelope) -> Vec<String> {
    if let Some(lines) = appfs_event_card_lines_from_terminal_render(envelope) {
        return lines;
    }

    let app_label = envelope.app_id.as_deref().unwrap_or("appfs");
    let mut lines = Vec::new();

    if envelope.input_type == "message.received" {
        let from = payload_string(envelope.payload.as_ref(), "from_display_name")
            .or_else(|| payload_string(envelope.payload.as_ref(), "from_principal"))
            .or_else(|| payload_string(envelope.payload.as_ref(), "contact_key"))
            .unwrap_or_else(|| "unknown".to_string());
        let mut meta = format!("{app_label} · message.received · from {from}");
        if envelope.requires_attention {
            meta.push_str(" · attention required");
        }
        lines.push(format!("\x1b[1;36m{meta}\x1b[0m"));

        let body = payload_string(envelope.payload.as_ref(), "text")
            .or_else(|| payload_string(envelope.payload.as_ref(), "text_preview"))
            .unwrap_or_else(|| single_line_preview(&envelope.text, 280));
        if !body.is_empty() {
            lines.push(body);
        }
        return lines;
    }

    let mut meta = format!("{app_label} · {}", envelope.input_type.trim());
    if let Some(principal) = &envelope.principal_id {
        meta.push_str(&format!(" · principal {principal}"));
    }
    if envelope.requires_attention {
        meta.push_str(" · attention required");
    }
    lines.push(format!("\x1b[1;36m{meta}\x1b[0m"));

    let preview = single_line_preview(&envelope.text, 280);
    if !preview.is_empty() {
        lines.push(preview);
    }

    lines
}

pub fn summarize_appfs_pending_input(envelope: &InputEnvelope) -> String {
    let app_label = envelope.app_id.as_deref().unwrap_or("AppFS");
    if envelope.input_type == "message.received" {
        let from = payload_string(envelope.payload.as_ref(), "from_display_name")
            .or_else(|| payload_string(envelope.payload.as_ref(), "from_principal"))
            .or_else(|| payload_string(envelope.payload.as_ref(), "contact_key"))
            .unwrap_or_else(|| "unknown".to_string());
        let body = payload_string(envelope.payload.as_ref(), "text")
            .or_else(|| payload_string(envelope.payload.as_ref(), "text_preview"))
            .unwrap_or_else(|| single_line_preview(&envelope.text, 160));
        let attention = if envelope.requires_attention {
            "needs attention; "
        } else {
            ""
        };
        return format!("{app_label} message from {from}: {attention}{body}");
    }

    let preview = single_line_preview(&envelope.text, 160);
    if preview.is_empty() {
        format!("{app_label} {}", envelope.input_type.trim())
    } else {
        format!("{app_label} {}: {preview}", envelope.input_type.trim())
    }
}

fn event_terminal_render(envelope: &InputEnvelope) -> Option<&Value> {
    let metadata = envelope.event_render_metadata.as_ref()?;
    metadata
        .get("terminal_render")
        .or_else(|| metadata.get("ui_render"))
        .or_else(|| metadata.get("user_render"))
}

fn appfs_event_card_lines_from_terminal_render(
    envelope: &InputEnvelope,
) -> Option<Vec<String>> {
    let render = event_terminal_render(envelope)?;
    let mode = render
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("card");
    if matches!(mode, "hidden" | "drop" | "debug_only") {
        return Some(Vec::new());
    }

    if let Some(lines) = render.get("lines").and_then(Value::as_array) {
        let rendered = lines
            .iter()
            .filter_map(Value::as_str)
            .map(|template| {
                render_event_template_for_target(
                    envelope,
                    template,
                    EventTemplateTarget::Terminal,
                )
            })
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        return Some(rendered);
    }

    let template = render.get("template").and_then(Value::as_str)?;
    let rendered = render_event_template_for_target(
        envelope,
        template,
        EventTemplateTarget::Terminal,
    );
    Some(rendered.lines().map(ToOwned::to_owned).collect())
}

fn payload_string(payload: Option<&Value>, key: &str) -> Option<String> {
    payload
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn single_line_preview(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = String::new();
    let mut count = 0usize;
    for ch in collapsed.chars() {
        if count >= max_chars {
            preview.push('…');
            return preview;
        }
        preview.push(ch);
        count += 1;
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appfs_event_card_lines_uses_terminal_render_lines() {
        let mut envelope = InputEnvelope::new(
            runtime::InputSource::AppfsEvent,
            "message.received",
            "fallback body",
        );
        envelope.app_id = Some("tinode".to_string());
        envelope.payload = Some(json!({
            "from_display_name": "AppFS Agent default",
            "text_preview": "hello"
        }));
        envelope.event_render_metadata = Some(json!({
            "terminal_render": {
                "mode": "card",
                "lines": [
                    "{{ansi.cyan}}{{app.display_name}} from {{message.sender}}{{ansi.reset}}",
                    "{{message.body}}"
                ]
            }
        }));

        let lines = appfs_event_card_lines(&envelope);

        assert_eq!(lines[0], "\x1b[36mTinode from AppFS Agent default\x1b[0m");
        assert_eq!(lines[1], "hello");
    }

    #[test]
    fn appfs_event_card_lines_falls_back_when_no_render_metadata() {
        let mut envelope = InputEnvelope::new(
            runtime::InputSource::AppfsEvent,
            "message.received",
            "fallback body",
        );
        envelope.app_id = Some("tinode".to_string());
        envelope.payload = Some(json!({
            "from_display_name": "AppFS Agent default",
            "text_preview": "hello"
        }));

        let lines = appfs_event_card_lines(&envelope);

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("tinode · message.received · from AppFS Agent default"));
        assert_eq!(lines[1], "hello");
    }
}
