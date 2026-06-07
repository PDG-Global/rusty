// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::app::{AppState, AutocompleteState, MessageRole};

pub fn draw(app: &AppState, area: Rect, buf: &mut Buffer) {
    // Dynamically size the input area based on content length.
    // 2 border rows + ceil(text_width / inner_width) content rows, clamped to [3..8].
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    let text_len = if app.input.is_empty() {
        28 // placeholder "Type a message or / for commands..."
    } else {
        app.input.len()
    };
    let content_rows = text_len.div_ceil(inner_width).max(1);
    let input_height = (content_rows as u16 + 2).clamp(3, 8);

    // Dynamically size the todo panel based on content
    let todo_height = if let Some(ref todos) = app.pinned_todos {
        let line_count = todos.lines().count() as u16;
        // 2 for borders + content lines, capped at 12
        (line_count + 2).min(12)
    } else {
        0
    };

    let mut constraints = vec![Constraint::Min(1)]; // Chat area
    if todo_height > 0 {
        constraints.push(Constraint::Length(todo_height)); // Todo panel
    }
    constraints.push(Constraint::Length(input_height)); // Input area
    constraints.push(Constraint::Length(1)); // Status bar

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    draw_chat(app, chunks[idx], buf);
    idx += 1;
    if todo_height > 0 {
        draw_todos(app, chunks[idx], buf);
        idx += 1;
    }
    let input_area_idx = idx;
    draw_input(app, chunks[idx], buf);
    idx += 1;
    draw_status(app, chunks[idx], buf);

    // Overlay autocomplete dropdown below input area
    if let Some(ref ac) = app.autocomplete {
        if !ac.matches.is_empty() {
            let input_area = chunks[input_area_idx];
            draw_autocomplete(
                ac,
                input_area.x + 1,
                input_area.y + input_area.height,
                input_area.width.saturating_sub(2),
                area,
                buf,
            );
        }
    }

    // Overlay session picker if active
    if app.session_picker.is_some() {
        let dim_style = Style::default().fg(Color::Gray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_style(dim_style);
            }
        }
        let picker_area = centered_rect(80, 16, area);
        draw_session_picker(app, picker_area, buf);
    }

    // Overlay file picker if active
    if app.file_picker.is_some() {
        let dim_style = Style::default().fg(Color::Gray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_style(dim_style);
            }
        }
        let picker_area = centered_rect(60, 20, area);
        draw_file_picker(app, picker_area, buf);
    }

    // Overlay settings if active
    if app.settings_overlay.is_some() {
        let dim_style = Style::default().fg(Color::Gray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_style(dim_style);
            }
        }
        draw_settings(app, area, buf);
    }

    // Overlay plan approval if active
    if app.plan_approval.is_some() {
        let dim_style = Style::default().fg(Color::Gray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_style(dim_style);
            }
        }
        let popup_area = centered_rect(75, area.height.saturating_sub(4), area);
        draw_plan_approval(app, popup_area, buf);
    }

    // Overlay model form if active (separate from settings overlay)
    if app.model_form.is_some() {
        let dim_style = Style::default().fg(Color::Gray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_style(dim_style);
            }
        }
        let form_area = centered_rect(60, 70, area);
        draw_model_form(app, form_area, buf);
    }
}

fn draw_chat(app: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray))
        .title(Span::styled("rusty", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));

    let inner = block.inner(area);
    block.render(area, buf);

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        let (prefix, base_style) = match msg.role {
            MessageRole::User => (
                "> ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            MessageRole::Assistant => ("  ", Style::default().fg(Color::White)),
            MessageRole::System => ("! ", Style::default().fg(Color::LightYellow)),
        };

        render_content(&msg.content, prefix, base_style, &mut lines, width);
        lines.push(Line::from(""));
    }

    // Thinking text — collapsed or expanded
    if app.is_thinking {
        let think_style = Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC);
        let count = app.thinking_text.lines().count();
        if app.thinking_expanded && !app.thinking_text.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  thinking... ({} lines, ctrl+o to collapse)", count),
                think_style,
            )));
            for line in app.thinking_text.lines() {
                let wrapped = wrap_text(&format!("    {line}"), width, "    ");
                for wline in wrapped {
                    lines.push(Line::from(Span::styled(
                        wline,
                        Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC),
                    )));
                }
            }
        } else {
            let label = if count > 0 {
                format!("  thinking... ({} lines, ctrl+o to expand)", count)
            } else {
                "  thinking...".to_string()
            };
            lines.push(Line::from(Span::styled(label, think_style)));
        }
        lines.push(Line::from(""));
    } else if app.thinking_line_count > 0 {
        let think_style = Style::default().fg(Color::Gray);
        if app.thinking_expanded && !app.saved_thinking.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  thought ({} lines, ctrl+o to collapse)", app.thinking_line_count),
                think_style,
            )));
            for line in app.saved_thinking.lines() {
                let wrapped = wrap_text(&format!("    {line}"), width, "    ");
                for wline in wrapped {
                    lines.push(Line::from(Span::styled(wline, think_style)));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                format!("  thought ({} lines, ctrl+o to expand)", app.thinking_line_count),
                think_style,
            )));
        }
        lines.push(Line::from(""));
    }

    // Streaming text
    if app.is_streaming && !app.streaming_text.is_empty() {
        render_content(&app.streaming_text, "  ", Style::default().fg(Color::White), &mut lines, width);
        lines.push(Line::from(Span::styled(
            "  \u{2588}",
            Style::default().fg(Color::Green),
        )));
    }

    // Inline permission prompt
    if let Some(ref prompt) = app.permission_prompt {
        use rusty_core::permissions::build_tool_description;
        let desc = build_tool_description(&prompt.request.tool_name, &prompt.request.raw_input);
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  \u{25B6} ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{} ", &prompt.request.tool_name),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Permission required ", Style::default().fg(Color::White)),
            Span::styled("[y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" Allow ", Style::default().fg(Color::Gray)),
            Span::styled("[n]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" Deny ", Style::default().fg(Color::Gray)),
            Span::styled("[a]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Session ", Style::default().fg(Color::Gray)),
            Span::styled("[d]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(" Always", Style::default().fg(Color::Gray)),
        ]));
    }

    // Scroll — prefer user's manual offset, otherwise auto-scroll to bottom
    let visible_height = inner.height as usize;
    let scroll = if app.scroll_offset > 0 {
        // User has scrolled up; start line = total - viewport - offset
        lines.len().saturating_sub(visible_height + app.scroll_offset)
    } else if lines.len() > visible_height {
        lines.len() - visible_height
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    Widget::render(&paragraph, inner, buf);
}

/// Render content with code blocks, tables, inline markdown, and word wrapping
fn render_content(content: &str, prefix: &str, base_style: Style, lines: &mut Vec<Line>, width: usize) {
    let msg_lines: Vec<&str> = content.lines().collect();
    let mut in_code_block = false;

    // Collect table rows for batch rendering
    let mut table_buf: Vec<Vec<String>> = Vec::new();
    let mut table_aligns: Vec<ColAlign> = Vec::new();

    for (i, line_text) in msg_lines.iter().enumerate() {
        let line_str = *line_text;

        // Tool indicator lines — color them distinctly
        if line_str.starts_with("  \u{25B6} ") {
            // Running tool ▶
            let wrapped = wrap_text(line_str, width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    wline,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        if line_str.starts_with("  \u{2713} ") {
            // Tool done ✓
            let wrapped = wrap_text(line_str, width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    wline,
                    Style::default().fg(Color::Green),
                )));
            }
            continue;
        }
        if line_str.starts_with("  \u{2717} ") {
            // Tool error ✗
            let wrapped = wrap_text(line_str, width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    wline,
                    Style::default().fg(Color::Red),
                )));
            }
            continue;
        }
        // Tool output lines (indented with ⎿ or …)
        if line_str.starts_with("    \u{257F} ") || line_str.starts_with("    \u{2026} ") {
            let wrapped = wrap_text(line_str, width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    wline,
                    Style::default().fg(Color::Gray),
                )));
            }
            continue;
        }

        // Toggle code blocks
        if line_str.starts_with("```") {
            // Flush any pending table
            if !table_buf.is_empty() {
                render_table(&table_buf, &table_aligns, base_style, lines, width);
                table_buf.clear();
                table_aligns.clear();
            }
            in_code_block = !in_code_block;
            let wrapped = wrap_text(&format!("  {line_str}"), width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(wline, Style::default().fg(Color::Gray))));
            }
            continue;
        }

        if in_code_block {
            let wrapped = wrap_text(&format!("  {line_str}"), width, "    ");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    wline,
                    Style::default().fg(Color::LightGreen),
                )));
            }
            continue;
        }

        // Detect table rows: line contains | and looks like a table
        if is_table_row(line_str) {
            // Capture alignment from separator rows (|---|:---:|---:|)
            if is_table_separator(line_str) {
                let inner = line_str
                    .trim()
                    .strip_prefix('|')
                    .unwrap_or(line_str.trim())
                    .strip_suffix('|')
                    .unwrap_or(line_str.trim());
                table_aligns = inner.split('|').map(parse_col_align).collect();
                continue;
            }
            let cells = parse_table_row(line_str);
            table_buf.push(cells);
            continue;
        }

        // Flush any pending table before rendering non-table content
        if !table_buf.is_empty() {
            render_table(&table_buf, &table_aligns, base_style, lines, width);
            table_buf.clear();
            table_aligns.clear();
        }

        // ATX headings: # through ######
        if let Some(heading) = parse_atx_heading(line_str) {
            let style = match heading.level {
                1 => base_style.fg(Color::White).add_modifier(Modifier::BOLD),
                2 => base_style.fg(Color::LightCyan).add_modifier(Modifier::BOLD),
                _ => base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
            };
            // Add blank line before heading (unless it's the first content)
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            let indent = if i == 0 { prefix } else { "  " };
            let mut spans = vec![Span::styled(indent.to_string(), base_style)];
            spans.extend(parse_inline_markdown(&heading.text, style));
            for wrapped_line in wrap_line(&spans, width, indent) {
                lines.push(Line::from(wrapped_line));
            }
            continue;
        }

        // Horizontal rules: ---, ***, ___
        if is_horizontal_rule(line_str) {
            let rule: String = "─".repeat(width.saturating_sub(4));
            lines.push(Line::from(Span::styled(
                format!("  {rule}"),
                Style::default().fg(Color::Gray),
            )));
            continue;
        }

        // Unordered list items: - , * , +
        if let Some(item_text) = parse_unordered_list_item(line_str) {
            let indent = if i == 0 { prefix } else { "  " };
            let mut spans = vec![Span::styled(indent.to_string(), base_style)];
            spans.push(Span::styled("• ", base_style));
            spans.extend(parse_inline_markdown(&item_text, base_style));
            for wrapped_line in wrap_line(&spans, width, indent) {
                lines.push(Line::from(wrapped_line));
            }
            continue;
        }

        // Ordered list items: 1. , 2. , etc.
        if let Some((num, item_text)) = parse_ordered_list_item(line_str) {
            let indent = if i == 0 { prefix } else { "  " };
            let mut spans = vec![Span::styled(indent.to_string(), base_style)];
            spans.push(Span::styled(format!("{num}. "), base_style));
            spans.extend(parse_inline_markdown(&item_text, base_style));
            for wrapped_line in wrap_line(&spans, width, indent) {
                lines.push(Line::from(wrapped_line));
            }
            continue;
        }

        // Blockquotes: >
        if let Some(quote_text) = parse_blockquote(line_str) {
            let quote_style = Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC);
            let indent = if i == 0 { prefix } else { "  " };
            let mut spans = vec![Span::styled(indent.to_string(), base_style)];
            spans.push(Span::styled("│ ", Style::default().fg(Color::Gray)));
            spans.extend(parse_inline_markdown(&quote_text, quote_style));
            for wrapped_line in wrap_line(&spans, width, indent) {
                lines.push(Line::from(wrapped_line));
            }
            continue;
        }

        // First line gets the prefix
        let indent = if i == 0 { prefix } else { "  " };
        let mut spans = vec![Span::styled(indent.to_string(), base_style)];
        spans.extend(parse_inline_markdown(line_str, base_style));
        // Wrap the line
        for wrapped_line in wrap_line(&spans, width, indent) {
            lines.push(Line::from(wrapped_line));
        }
    }

    // Flush any remaining table
    if !table_buf.is_empty() {
        render_table(&table_buf, &table_aligns, base_style, lines, width);
    }
}

/// Parsed ATX heading info
struct HeadingInfo {
    level: usize,
    text: String,
}

/// Parse an ATX heading line (# through ######).
/// Returns Some(HeadingInfo) if the line is a heading.
fn parse_atx_heading(line: &str) -> Option<HeadingInfo> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let mut level = 0;
    for ch in trimmed.chars() {
        if ch == '#' {
            level += 1;
        } else {
            break;
        }
    }
    if level > 6 || level == 0 {
        return None;
    }
    let rest = &trimmed[level..];
    if rest.starts_with(' ') {
        Some(HeadingInfo {
            level,
            text: rest[1..].to_string(),
        })
    } else {
        None
    }
}

/// Check if a line is a horizontal rule (---, ***, ___).
fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let ch = trimmed.chars().next().unwrap();
    if ch != '-' && ch != '*' && ch != '_' {
        return false;
    }
    trimmed.chars().all(|c| c == ch || c == ' ')
}

/// Parse an unordered list item (- , * , + ).
/// Returns Some(text) if the line is a list item.
fn parse_unordered_list_item(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'-' || bytes[0] == b'*' || bytes[0] == b'+') && bytes[1] == b' ' {
            return Some(trimmed[2..].to_string());
        }
    }
    None
}

/// Parse an ordered list item (1. , 2. , etc.).
/// Returns Some((number, text)) if the line is an ordered list item.
fn parse_ordered_list_item(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    let mut num_end = 0;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            num_end += ch.len_utf8();
        } else {
            break;
        }
    }
    if num_end > 0 && num_end < trimmed.len() && trimmed.as_bytes()[num_end] == b'.' {
        let num = trimmed[..num_end].to_string();
        let rest = &trimmed[num_end + 1..];
        if rest.starts_with(' ') {
            return Some((num, rest[1..].to_string()));
        }
    }
    None
}

/// Parse a blockquote line (> text).
/// Returns Some(text) if the line is a blockquote.
fn parse_blockquote(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('>') {
        let rest = &trimmed[1..];
        if rest.starts_with(' ') {
            return Some(rest[1..].to_string());
        } else if rest.is_empty() {
            return Some(String::new());
        }
    }
    None
}

/// Wrap plain text to fit within `width` columns. Returns Vec of strings.
/// `continuation_indent` is prepended to continuation lines.
fn wrap_text(text: &str, width: usize, continuation_indent: &str) -> Vec<String> {
    if width == 0 || text.len() <= width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        let avail = if result.is_empty() { width } else { width - continuation_indent.len() };

        if pos + avail >= chars.len() {
            let line: String = chars[pos..].iter().collect();
            if result.is_empty() {
                result.push(line);
            } else {
                result.push(format!("{continuation_indent}{line}"));
            }
            break;
        }

        let end = pos + avail;
        let mut break_pos = None;
        for j in (pos..end).rev() {
            if chars[j] == ' ' {
                break_pos = Some(j);
                break;
            }
        }

        if let Some(bp) = break_pos {
            let line: String = chars[pos..bp].iter().collect();
            if result.is_empty() {
                result.push(line);
            } else {
                result.push(format!("{continuation_indent}{line}"));
            }
            pos = bp + 1;
        } else {
            let line: String = chars[pos..end].iter().collect();
            if result.is_empty() {
                result.push(line);
            } else {
                result.push(format!("{continuation_indent}{line}"));
            }
            pos = end;
        }
    }

    if result.is_empty() {
        result.push(text.to_string());
    }

    result
}

/// Wrap a styled Line (Vec of Spans) to fit within `width` columns.
/// Returns Vec of Vec<Span> (one per wrapped line).
fn wrap_line(spans: &[Span<'static>], width: usize, continuation_indent: &str) -> Vec<Vec<Span<'static>>> {
    // Flatten to (char, style) pairs for wrapping
    let mut chars_with_style: Vec<(char, Style)> = Vec::new();
    for span in spans {
        let style = span.style;
        for ch in span.content.chars() {
            chars_with_style.push((ch, style));
        }
    }

    if chars_with_style.len() <= width {
        return vec![spans.to_vec()];
    }

    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut pos = 0;

    while pos < chars_with_style.len() {
        let avail = if result.is_empty() { width } else { width - continuation_indent.len() };

        if pos + avail >= chars_with_style.len() {
            // Rest fits
            let mut line_spans = Vec::new();
            if !result.is_empty() {
                line_spans.push(Span::raw(continuation_indent.to_string()));
            }
            line_spans.extend(build_spans_from_chars(&chars_with_style[pos..]));
            result.push(line_spans);
            break;
        }

        let end = pos + avail;
        // Find last space
        let mut break_pos = None;
        for j in (pos..end).rev() {
            if chars_with_style[j].0 == ' ' {
                break_pos = Some(j);
                break;
            }
        }

        let bp = break_pos.unwrap_or(end);
        let mut line_spans = Vec::new();
        if !result.is_empty() {
            line_spans.push(Span::raw(continuation_indent.to_string()));
        }
        line_spans.extend(build_spans_from_chars(&chars_with_style[pos..bp]));
        result.push(line_spans);

        pos = if break_pos.is_some() { bp + 1 } else { bp };
    }

    if result.is_empty() {
        result.push(spans.to_vec());
    }

    result
}

/// Build Vec<Span> from (char, style) pairs, merging adjacent same-style chars.
fn build_spans_from_chars(chars: &[(char, Style)]) -> Vec<Span<'static>> {
    if chars.is_empty() {
        return vec![];
    }

    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_style = chars[0].1;

    for (ch, style) in chars {
        if *style == current_style {
            current.push(*ch);
        } else {
            if !current.is_empty() {
                spans.push(Span::styled(current.clone(), current_style));
            }
            current = ch.to_string();
            current_style = *style;
        }
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, current_style));
    }

    spans
}

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    // Must start with | and have at least 2 |
    trimmed.starts_with('|') && trimmed.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    // Looks like |---|---| or | --- | --- | or |:---:|---:|
    trimmed.starts_with('|')
        && trimmed
            .chars()
            .all(|c| "|- \t:".contains(c))
        && trimmed.contains('-')
}

/// Return the display width of a character (CJK = 2, others = 1)
fn char_display_width(ch: char) -> usize {
    let cp = ch as u32;
    // CJK Unified Ideographs, CJK Compat, CJK Ext A/B, etc.
    if (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x20000..=0x2A6DF).contains(&cp)
        || (0x2A700..=0x2B73F).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0x2F800..=0x2FA1F).contains(&cp)
        // Fullwidth forms
        || (0xFF01..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        // CJK punctuation
        || (0x3000..=0x303F).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
    {
        2
    } else {
        1
    }
}

/// Display width of a string (counts CJK as 2)
fn str_display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

/// Alignment for a table column
#[derive(Clone, Copy, Debug, PartialEq)]
enum ColAlign {
    Left,
    Center,
    Right,
}

/// Parse alignment from a separator cell like "---", ":---", "---:", ":---:"
fn parse_col_align(cell: &str) -> ColAlign {
    let trimmed = cell.trim();
    let left = trimmed.starts_with(':');
    let right = trimmed.ends_with(':');
    match (left, right) {
        (true, true) => ColAlign::Center,
        (false, true) => ColAlign::Right,
        _ => ColAlign::Left,
    }
}

fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    // Remove leading/trailing |
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed);
    inner
        .split('|')
        .map(|s| s.trim().to_string())
        .collect()
}

fn render_table(
    rows: &[Vec<String>],
    aligns: &[ColAlign],
    base_style: Style,
    lines: &mut Vec<Line>,
    term_width: usize,
) {
    if rows.is_empty() {
        return;
    }

    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return;
    }

    // ── Measure natural column widths (display-width, not byte-len) ──
    // Strip inline markdown markers (**bold** → bold) for accurate width
    let mut natural_widths = vec![1usize; num_cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                let stripped = strip_inline_markdown(cell);
                let w = str_display_width(&stripped);
                natural_widths[i] = natural_widths[i].max(w);
            }
        }
    }

    // ── Distribute columns to fit terminal width ──
    // Budget: 2 indent + num_cols*(2 padding + 1 separator) + (num_cols-1) interior borders + 2 outer borders
    // Simplified: 2 + num_cols*3 + (num_cols-1) = 4*num_cols + 1
    let border_budget = 4 * num_cols + 1;
    let available = term_width.saturating_sub(border_budget).max(num_cols * 3);
    let total_natural: usize = natural_widths.iter().sum();

    let mut col_widths: Vec<usize>;
    if total_natural <= available {
        // All columns fit naturally
        col_widths = natural_widths;
    } else {
        // Scale proportionally to fit, minimum 3 per column
        col_widths = natural_widths
            .iter()
            .map(|&w| {
                let scaled = (w as f64 / total_natural as f64 * available as f64) as usize;
                scaled.max(3)
            })
            .collect();

        // Fix rounding drift: add/remove from widest column
        let current_total: usize = col_widths.iter().sum();
        if current_total != available && !col_widths.is_empty() {
            let widest = col_widths
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| *w)
                .map(|(i, _)| i)
                .unwrap_or(0);
            if current_total < available {
                col_widths[widest] += available - current_total;
            } else {
                col_widths[widest] = col_widths[widest].saturating_sub(current_total - available);
            }
        }
    }

    // ── Helper: build a horizontal border line ──
    let border = |left: char, mid: char, right: char, fill: char| -> String {
        let mut s = format!("  {}", left);
        for (i, w) in col_widths.iter().enumerate() {
            s.push_str(&fill.to_string().repeat(w + 2));
            if i < num_cols - 1 {
                s.push(mid);
            }
        }
        s.push(right);
        s
    };

    let gray = Style::default().fg(Color::Gray);

    // Top border ┌─┬─┐
    lines.push(Line::from(Span::styled(
        border('\u{250C}', '\u{252C}', '\u{2510}', '\u{2500}'),
        gray,
    )));

    // ── Render each row ──
    for (row_idx, row) in rows.iter().enumerate() {
        let mut spans = vec![Span::styled("  \u{2502}".to_string(), gray)]; // │

        for col in 0..num_cols {
            let cell = row.get(col).map(|s| s.as_str()).unwrap_or("");
            let w = col_widths[col];
            let align = aligns.get(col).copied().unwrap_or(ColAlign::Left);

            // Truncate to fit (by display width, ignoring markdown markers)
            let truncated: String = {
                let mut out = String::new();
                let mut used = 0usize;
                let chars: Vec<char> = cell.chars().collect();
                let len = chars.len();
                let mut i = 0;
                while i < len {
                    // Check for markdown markers — if they fit within width, keep them
                    // but only count the visible content width
                    let marker_len;
                    let visible;
                    if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
                        if let Some(end) = find_closing(&chars, i + 2, "**") {
                            let visible_text: String = chars[i + 2..end].iter().collect();
                            let vis_w = str_display_width(&visible_text);
                            if used + vis_w > w { break; }
                            out.extend(chars[i..=end + 1].iter());
                            used += vis_w;
                            i = end + 2;
                            continue;
                        }
                        marker_len = 0; visible = 0; // fall through
                    } else if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*') {
                        if let Some(end) = find_closing(&chars, i + 1, "*") {
                            let visible_text: String = chars[i + 1..end].iter().collect();
                            let vis_w = str_display_width(&visible_text);
                            if used + vis_w > w { break; }
                            out.extend(chars[i..=end].iter());
                            used += vis_w;
                            i = end + 1;
                            continue;
                        }
                        marker_len = 0; visible = 0;
                    } else if chars[i] == '_' {
                        if let Some(end) = find_closing(&chars, i + 1, "_") {
                            let visible_text: String = chars[i + 1..end].iter().collect();
                            let vis_w = str_display_width(&visible_text);
                            if used + vis_w > w { break; }
                            out.extend(chars[i..=end].iter());
                            used += vis_w;
                            i = end + 1;
                            continue;
                        }
                        marker_len = 0; visible = 0;
                    } else if chars[i] == '`' {
                        if let Some(end) = find_closing(&chars, i + 1, "`") {
                            let visible_text: String = chars[i + 1..end].iter().collect();
                            let vis_w = str_display_width(&visible_text);
                            if used + vis_w > w { break; }
                            out.extend(chars[i..=end].iter());
                            used += vis_w;
                            i = end + 1;
                            continue;
                        }
                        marker_len = 0; visible = 0;
                    } else {
                        marker_len = 0; visible = 0;
                    }
                    let _ = (marker_len, visible);
                    let cw = char_display_width(chars[i]);
                    if used + cw > w { break; }
                    out.push(chars[i]);
                    used += cw;
                    i += 1;
                }
                out
            };

            // Parse inline markdown for styled rendering
            let cell_spans = parse_inline_markdown(&truncated, base_style);
            let cell_vis_w: usize = cell_spans.iter().map(|s| str_display_width(&s.content)).sum();

            // Build cell with padding applied to the last span
            let pad_needed = w.saturating_sub(cell_vis_w);
            match align {
                ColAlign::Left => {
                    spans.push(Span::styled(" ".to_string(), base_style));
                    for span in &cell_spans {
                        spans.push(span.clone());
                    }
                    spans.push(Span::styled(" ".repeat(pad_needed + 1), base_style));
                }
                ColAlign::Right => {
                    spans.push(Span::styled(" ".repeat(pad_needed + 1), base_style));
                    for span in &cell_spans {
                        spans.push(span.clone());
                    }
                    spans.push(Span::styled(" ".to_string(), base_style));
                }
                ColAlign::Center => {
                    let left = pad_needed / 2 + 1;
                    let right = pad_needed - pad_needed / 2 + 1;
                    spans.push(Span::styled(" ".repeat(left), base_style));
                    for span in &cell_spans {
                        spans.push(span.clone());
                    }
                    spans.push(Span::styled(" ".repeat(right), base_style));
                }
            }

            // Separator: border (cell padding is already included above)
            spans.push(Span::styled("\u{2502}".to_string(), gray)); // │
        }
        lines.push(Line::from(spans));

        // Separator after header row
        if row_idx == 0 {
            lines.push(Line::from(Span::styled(
                border('\u{251C}', '\u{253C}', '\u{2524}', '\u{2500}'), // ├┼┤─
                gray,
            )));
        }
    }

    // Bottom border └─┴─┘
    lines.push(Line::from(Span::styled(
        border('\u{2514}', '\u{2534}', '\u{2518}', '\u{2500}'),
        gray,
    )));
}

/// Parse inline markdown: **bold**, *italic*, `code`, _italic_
fn parse_inline_markdown(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut current = String::new();

    while i < len {
        // **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !current.is_empty() {
                spans.push(Span::styled(current.clone(), base_style));
                current.clear();
            }
            let start = i + 2;
            let end = find_closing(&chars, start, "**");
            if let Some(end) = end {
                let bold_text: String = chars[start..end].iter().collect();
                spans.push(Span::styled(
                    bold_text,
                    base_style.add_modifier(Modifier::BOLD),
                ));
                i = end + 2;
                continue;
            }
        }

        // *italic*
        if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*') {
            if !current.is_empty() {
                spans.push(Span::styled(current.clone(), base_style));
                current.clear();
            }
            let start = i + 1;
            let end = find_closing(&chars, start, "*");
            if let Some(end) = end {
                let italic_text: String = chars[start..end].iter().collect();
                spans.push(Span::styled(
                    italic_text,
                    base_style.add_modifier(Modifier::ITALIC),
                ));
                i = end + 1;
                continue;
            }
        }

        // _italic_
        if chars[i] == '_' {
            if !current.is_empty() {
                spans.push(Span::styled(current.clone(), base_style));
                current.clear();
            }
            let start = i + 1;
            let end = find_closing(&chars, start, "_");
            if let Some(end) = end {
                let italic_text: String = chars[start..end].iter().collect();
                spans.push(Span::styled(
                    italic_text,
                    base_style.add_modifier(Modifier::ITALIC),
                ));
                i = end + 1;
                continue;
            }
        }

        // `inline code`
        if chars[i] == '`' {
            if !current.is_empty() {
                spans.push(Span::styled(current.clone(), base_style));
                current.clear();
            }
            let start = i + 1;
            let end = find_closing(&chars, start, "`");
            if let Some(end) = end {
                let code_text: String = chars[start..end].iter().collect();
                spans.push(Span::styled(
                    code_text,
                    Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
                ));
                i = end + 1;
                continue;
            }
        }

        current.push(chars[i]);
        i += 1;
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, base_style));
    }

    spans
}

fn find_closing(chars: &[char], start: usize, delim: &str) -> Option<usize> {
    let delim_chars: Vec<char> = delim.chars().collect();
    let delim_len = delim_chars.len();
    for i in start..chars.len() {
        if i + delim_len <= chars.len() && chars[i..i + delim_len] == *delim_chars
            && i > start {
                return Some(i);
            }
    }
    None
}

/// Strip inline markdown markers (**, *, _, `) for display-width calculation.
fn strip_inline_markdown(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut out = String::new();

    while i < len {
        // **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, "**") {
                out.extend(chars[i + 2..end].iter());
                i = end + 2;
                continue;
            }
        }
        // *italic*
        if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*') {
            if let Some(end) = find_closing(&chars, i + 1, "*") {
                out.extend(chars[i + 1..end].iter());
                i = end + 1;
                continue;
            }
        }
        // _italic_
        if chars[i] == '_' {
            if let Some(end) = find_closing(&chars, i + 1, "_") {
                out.extend(chars[i + 1..end].iter());
                i = end + 1;
                continue;
            }
        }
        // `code`
        if chars[i] == '`' {
            if let Some(end) = find_closing(&chars, i + 1, "`") {
                out.extend(chars[i + 1..end].iter());
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn draw_todos(app: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(Span::styled(
            " ☑ Tasks ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    ratatui::widgets::Widget::render(block, area, buf);

    if let Some(ref todos) = app.pinned_todos {
        let mut lines: Vec<Line> = Vec::new();
        for todo_line in todos.lines() {
            let styled = if todo_line.contains("[x]") || todo_line.contains("[X]") {
                Line::from(Span::styled(
                    format!("  {todo_line}"),
                    Style::default().fg(Color::Green),
                ))
            } else if todo_line.contains("[~]") {
                Line::from(Span::styled(
                    format!("  {todo_line}"),
                    Style::default().fg(Color::Yellow),
                ))
            } else if todo_line.contains("[ ]") {
                Line::from(Span::styled(
                    format!("  {todo_line}"),
                    Style::default().fg(Color::White),
                ))
            } else {
                Line::from(Span::styled(
                    format!("  {todo_line}"),
                    Style::default().fg(Color::DarkGray),
                ))
            };
            lines.push(styled);
        }

        // Render the lines, trimming from top if they exceed available space
        let visible_height = inner.height as usize;
        let start = if lines.len() > visible_height {
            lines.len() - visible_height
        } else {
            0
        };
        for (i, line) in lines[start..].iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            buf.set_line(inner.x, inner.y + i as u16, line, inner.width);
        }
    }
}

fn draw_input(app: &AppState, area: Rect, buf: &mut Buffer) {
    let is_slash = app.input.starts_with('/');
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_slash {
            Color::Magenta
        } else {
            Color::Green
        }))
        .title(Span::styled(
            if is_slash { " command " } else { " input " },
            Style::default().fg(if is_slash {
                Color::Magenta
            } else {
                Color::Green
            }).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    block.render(area, buf);

    // Render input text with a visible block cursor at cursor_pos
    let mut spans: Vec<Span<'_>> = Vec::new();
    if app.input.is_empty() {
        spans.push(Span::styled(
            "Type a message or / for commands...",
            Style::default().fg(Color::Gray),
        ));
    } else {
        let (text, style) = if is_slash {
            (app.input.clone(), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
        } else {
            (app.input.clone(), Style::default().fg(Color::White))
        };
        let cursor_pos = app.cursor_pos.min(text.len());
        // Text before cursor
        if cursor_pos > 0 {
            spans.push(Span::styled(text[..cursor_pos].to_string(), style));
        }
        // Cursor character (block cursor via reverse video)
        let cursor_style = style.add_modifier(Modifier::REVERSED);
        if cursor_pos < text.len() {
            let ch = text[cursor_pos..].chars().next().unwrap();
            spans.push(Span::styled(ch.to_string(), cursor_style));
            // Text after cursor
            let after = cursor_pos + ch.len_utf8();
            if after < text.len() {
                spans.push(Span::styled(text[after..].to_string(), style));
            }
        } else {
            // Cursor at end — render a space with reverse to show block cursor
            spans.push(Span::styled(" ", cursor_style));
        }
    }

    // Calculate which row the cursor is on to scroll to it
    let width = inner.width.max(1) as usize;
    let cursor_row = app.cursor_pos / width;
    let visible_rows = inner.height as usize;
    let scroll = (cursor_row + 1).saturating_sub(visible_rows);

    let paragraph = Paragraph::new(Line::from(spans))
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    Widget::render(&paragraph, inner, buf);
}

fn draw_status(app: &AppState, area: Rect, buf: &mut Buffer) {
    let model_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let separator_style = Style::default().fg(Color::Gray);
    let state_style = if app.is_streaming {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let state_text = if app.is_streaming { "streaming" } else { "ready" };

    // Context-window usage warning
    let context_window = app.status.context_window;
    let current_context = app.status.current_context_tokens;
    let usage_pct = if context_window > 0 {
        (current_context as f64 / context_window as f64 * 100.0) as u32
    } else {
        0
    };
    let token_style = if usage_pct >= 90 {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if usage_pct >= 75 {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let cached_style = Style::default().fg(Color::DarkGray);
    let cached_span = if app.status.cached_input_tokens > 0 {
        Span::styled(format!("| cached: {} ", app.status.cached_input_tokens), cached_style)
    } else {
        Span::styled("", cached_style)
    };

    let think_style = Style::default().fg(Color::Gray);
    let think_span = match app.status.thinking_level {
        Some(level) => Span::styled(format!("| thinking: {level}"), think_style),
        None => Span::styled("", think_style),
    };

    let cwd_display = app
        .working_dir
        .as_deref()
        .map(|d| {
            // Show a short version: use ~ for home dir
            let display = match std::env::var("HOME") {
                Ok(home) => {
                    let home_prefix = format!("{}/", home);
                    match d.strip_prefix(&home_prefix) {
                        Some(rel) => format!("~/{}", rel),
                        None => d.to_string(),
                    }
                }
                Err(_) => d.to_string(),
            };
            // Truncate if too long
            if display.len() > 40 {
                let safe = display.floor_char_boundary(display.len() - 39);
                format!("…{}", &display[safe..])
            } else {
                display
            }
        })
        .unwrap_or_default();

    let cwd_span = if cwd_display.is_empty() {
        Span::styled("", separator_style)
    } else {
        Span::styled(format!("| {cwd_display}"), separator_style)
    };

    let update_span = match &app.update_available {
        Some(ver) => Span::styled(
            format!("| update: {ver} "),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        None => Span::styled("", separator_style),
    };

    let spans = vec![
        Span::styled(format!(" {} ", app.status.model), model_style),
        Span::styled("| ", separator_style),
        Span::styled(format!("context: {}/{} ({}%) ", current_context, context_window, usage_pct.min(999)), token_style),
        cached_span,
        Span::styled("| ", separator_style),
        Span::styled(format!("{state_text} "), state_style),
        think_span,
        cwd_span,
        update_span,
    ];
    let paragraph = Paragraph::new(Line::from(spans));
    Widget::render(&paragraph, area, buf);
}

/// Create a centered rectangle with the given percentage width and fixed height.
/// Calculate the display width of a string in terminal columns.
/// Handles wide characters (CJK, emoji) that occupy 2 columns.
fn unicode_display_width(s: &str) -> usize {
    s.chars().map(|c| {
        // Common wide Unicode ranges: CJK, fullwidth, some emoji
        let cp = c as u32;
        if (0x1100..=0x115F).contains(&cp)
            || (0x2E80..=0x303E).contains(&cp)
            || (0x3040..=0x33BF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x4E00..=0x9FFF).contains(&cp)
            || (0xA000..=0xA4CF).contains(&cp)
            || (0xAC00..=0xD7AF).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
            || (0xFE10..=0xFE6F).contains(&cp)
            || (0xFF01..=0xFF60).contains(&cp)
            || (0xFFE0..=0xFFE6).contains(&cp)
            || (0x20000..=0x2FFFD).contains(&cp)
            || (0x30000..=0x3FFFD).contains(&cp)
            || (0x1F000..=0x1FAFF).contains(&cp)  // emoji
            || (0x2600..=0x27BF).contains(&cp)     // misc symbols
            || (0xFE00..=0xFE0F).contains(&cp)     // variation selectors
        {
            2
        } else {
            1
        }
    }).sum()
}

/// Truncate a string to fit within a given display width (terminal columns).
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for c in s.chars() {
        let cp = c as u32;
        let w = if (0x1100..=0x115F).contains(&cp)
            || (0x2E80..=0x303E).contains(&cp)
            || (0x3040..=0x33BF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x4E00..=0x9FFF).contains(&cp)
            || (0xA000..=0xA4CF).contains(&cp)
            || (0xAC00..=0xD7AF).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
            || (0xFE10..=0xFE6F).contains(&cp)
            || (0xFF01..=0xFF60).contains(&cp)
            || (0xFFE0..=0xFFE6).contains(&cp)
            || (0x20000..=0x2FFFD).contains(&cp)
            || (0x30000..=0x3FFFD).contains(&cp)
            || (0x1F000..=0x1FAFF).contains(&cp)
            || (0x2600..=0x27BF).contains(&cp)
            || (0xFE00..=0xFE0F).contains(&cp)
        {
            2
        } else {
            1
        };
        if width + w > max_width {
            break;
        }
        result.push(c);
        width += w;
    }
    result
}

fn draw_plan_approval(app: &AppState, area: Rect, buf: &mut Buffer) {
    let approval = match &app.plan_approval {
        Some(a) => a,
        None => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Plan Approval ",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    block.render(area, buf);

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "The LLM proposed a plan. Review and approve to proceed.",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Plan content
    render_content(&approval.plan_text, "  ", Style::default().fg(Color::White), &mut lines, width);

    // Footer with key hints
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("    ", Style::default()),
        Span::styled("[y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" Approve  ", Style::default().fg(Color::Gray)),
        Span::styled("[n]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled(" Reject  ", Style::default().fg(Color::Gray)),
        Span::styled("[j/k]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" Scroll", Style::default().fg(Color::Gray)),
    ]));

    // Scroll
    let visible_height = inner.height as usize;
    let scroll = if approval.scroll_offset > 0 {
        lines.len().saturating_sub(visible_height + approval.scroll_offset)
    } else if lines.len() > visible_height {
        lines.len() - visible_height
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    Widget::render(&paragraph, inner, buf);
}

fn draw_settings(app: &AppState, area: Rect, buf: &mut Buffer) {
    let settings = match &app.settings_overlay {
        Some(s) => s,
        None => return,
    };

    let popup = centered_rect(70, 22, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Settings ");

    let inner = block.inner(popup);
    block.render(popup, buf);

    if inner.height < 3 || inner.width < 10 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Tab bar
    let models_style = if settings.active_tab == crate::app::SettingsTab::Models {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let general_style = if settings.active_tab == crate::app::SettingsTab::General {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    lines.push(Line::from(vec![
        Span::styled(" Models ", models_style),
        Span::raw("  "),
        Span::styled(" General ", general_style),
    ]));
    lines.push(Line::from(Span::styled(
        "\u{2500}".repeat(inner.width as usize),
        Style::default().fg(Color::DarkGray),
    )));

    let content_height = (inner.height as usize).saturating_sub(lines.len() + 1);

    match settings.active_tab {
        crate::app::SettingsTab::Models => {
            let display_rows = settings.display_rows();
            let start = settings.scroll.min(display_rows.len().saturating_sub(1));
            let end = (start + content_height).min(display_rows.len());

            for row in &display_rows[start..end] {
                match row {
                    crate::app::DisplayRow::GroupHeader { name, count: _ } => {
                        lines.push(Line::from(Span::styled(
                            format!("  {name}"),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                    crate::app::DisplayRow::ModelEntry(idx) => {
                        let entry = &settings.models[*idx];
                        let is_selected = *idx == settings.selected;
                        let is_active = entry.name == settings.active_model_name;
                        let cursor = if is_selected { ">" } else { " " };
                        let active_marker = if is_active { "●" } else { " " };
                        let style = if is_selected {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        let marker_style = if is_selected {
                            style
                        } else if is_active {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        let name_style = if is_selected {
                            style
                        } else if is_active {
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {cursor}"), style),
                            Span::styled(format!("{active_marker} "), marker_style),
                            Span::styled(format!("{:<24}", entry.name), name_style),
                            Span::styled(
                                format!("{:<16}", entry.provider),
                                if is_selected {
                                    style
                                } else {
                                    Style::default().fg(Color::Gray)
                                },
                            ),
                            Span::styled(
                                format!("{} · {}", entry.model, entry.api_base),
                                if is_selected {
                                    style
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            ),
                        ]));
                    }
                }
            }
        }
        crate::app::SettingsTab::General => {
            lines.push(Line::from(""));

            let general_rows = [
                ("Thinking level", crate::model_registry::thinking_level_label(settings.general_thinking_level)),
                ("Permission mode", crate::app::permission_mode_label(settings.general_permission_mode)),
            ];

            for (i, (label, value)) in general_rows.iter().enumerate() {
                let is_selected = i == settings.general_selected;
                let cursor = if is_selected { ">" } else { " " };
                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {cursor} "), style),
                    Span::styled(format!("{:<24}", *label), style),
                    Span::styled(
                        value.to_string(),
                        if is_selected {
                            style
                        } else {
                            Style::default().fg(Color::Green)
                        },
                    ),
                ]));
            }
        }
    }

    lines.push(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Select  "),
        Span::styled("[Tab]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Switch Tab  "),
        Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Close"),
    ]));

    let paragraph = Paragraph::new(lines);
    Widget::render(&paragraph, inner, buf);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_height = height.min(area.height);
    let popup_width = (area.width * percent_x / 100).min(area.width);
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(area.x + x, area.y + y, popup_width, popup_height)
}

fn draw_session_picker(app: &AppState, area: Rect, buf: &mut Buffer) {
    let picker = match &app.session_picker {
        Some(p) => p,
        None => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Resume Session ");

    let inner = block.inner(area);
    block.render(area, buf);

    if picker.sessions.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No saved sessions found.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw(" Close"),
            ]),
        ];
        let paragraph = Paragraph::new(lines);
        Widget::render(&paragraph, inner, buf);
        return;
    }

    let visible_rows = inner.height.saturating_sub(3) as usize; // room for header + footer
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "  ID        ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Messages  ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Model           ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Updated          ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Preview",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ]));

    let end = (picker.scroll_offset + visible_rows).min(picker.sessions.len());
    for (i, session) in picker.sessions[picker.scroll_offset..end].iter().enumerate() {
        let actual_idx = picker.scroll_offset + i;
        let is_selected = actual_idx == picker.selected;

        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };

        let id_display = if session.id.len() >= 8 {
            &session.id[..8]
        } else {
            &session.id
        };

        let model_display = if session.model.len() > 14 {
            let safe = session.model.floor_char_boundary(14);
            format!("{}...", &session.model[..safe])
        } else {
            format!("{:14}", session.model)
        };

        let preview_display = if session.preview.len() > 30 {
            let safe = session.preview.floor_char_boundary(30);
            format!("{}...", &session.preview[..safe])
        } else {
            session.preview.clone()
        };

        let cursor = if is_selected { "> " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(
                format!("{cursor}{id_display:<8}  ",),
                style,
            ),
            Span::styled(
                format!("{:<8}  ", session.message_count),
                style,
            ),
            Span::styled(
                format!("{model_display}  ",),
                style,
            ),
            Span::styled(
                format!("{:<16}  ", session.updated_at),
                style,
            ),
            Span::styled(preview_display, style),
        ]));
    }

    // Scroll indicator
    if picker.sessions.len() > visible_rows {
        let scroll_pct = if picker.sessions.len() > 1 {
            (picker.selected * 100) / (picker.sessions.len() - 1)
        } else {
            100
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  {}/{} ({}%)",
                picker.selected + 1,
                picker.sessions.len(),
                scroll_pct
            ),
            Style::default().fg(Color::Gray),
        )));
    }

    // Footer
    lines.push(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Resume  "),
        Span::styled("[Up/Down]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Navigate  "),
        Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Cancel"),
    ]));

    let paragraph = Paragraph::new(lines);
    Widget::render(&paragraph, inner, buf);
}

fn draw_file_picker(app: &AppState, area: Rect, buf: &mut Buffer) {
    let picker = match &app.file_picker {
        Some(p) => p,
        None => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" File Reference ");

    let inner = block.inner(area);
    block.render(area, buf);

    let mut lines: Vec<Line> = Vec::new();

    // Query line
    let query_display = if picker.query.is_empty() {
        String::from("type to search...")
    } else {
        picker.query.clone()
    };
    let query_style = if picker.query.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    lines.push(Line::from(Span::styled(format!("  @{query_display}"), query_style)));
    lines.push(Line::from(""));

    if picker.matches.is_empty() {
        if picker.query.is_empty() {
            lines.push(Line::from(Span::styled(
                "  Start typing to search files...",
                Style::default().fg(Color::Gray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  No matching files found.",
                Style::default().fg(Color::Gray),
            )));
        }
    } else {
        // Calculate visible rows based on available space
        let visible_rows = inner.height.saturating_sub(3) as usize; // room for query + divider + footer
        let end = (picker.scroll_offset + visible_rows).min(picker.matches.len());

        for (i, entry) in picker.matches[picker.scroll_offset..end].iter().enumerate() {
            let actual_idx = picker.scroll_offset + i;
            let is_selected = actual_idx == picker.selected;

            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            let icon = if entry.is_dir { "▸ " } else { "  " };
            let cursor = if is_selected { "> " } else { "  " };
            let mut display_path = entry.display.clone();
            if entry.is_dir {
                display_path.push('/');
            }

            // Truncate if too long
            let prefix_width = 5usize; // cursor(2) + icon(2) + space(1)
            let max_path_cols = inner.width.saturating_sub(prefix_width as u16) as usize;
            let path_cols = unicode_display_width(&display_path);
            if path_cols > max_path_cols {
                display_path = truncate_to_width(&display_path, max_path_cols.saturating_sub(3));
                display_path.push_str("...");
            }

            lines.push(Line::from(vec![
                Span::styled(format!("{cursor}{icon}"), style),
                Span::styled(display_path, style),
            ]));
        }

        // Scroll indicator
        if picker.matches.len() > visible_rows {
            let scroll_pct = if picker.matches.len() > 1 {
                (picker.selected * 100) / (picker.matches.len() - 1)
            } else {
                100
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}/{} ({}%)",
                    picker.selected + 1,
                    picker.matches.len(),
                    scroll_pct
                ),
                Style::default().fg(Color::Gray),
            )));
        }
    }

    // Footer
    lines.push(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Select  "),
        Span::styled("[Up/Down]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Navigate  "),
        Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Cancel"),
    ]));

    let paragraph = Paragraph::new(lines);
    Widget::render(&paragraph, inner, buf);
}

/// Draw the model add/edit form as a centered popup.
fn draw_model_form(app: &AppState, area: Rect, buf: &mut Buffer) {
    use crate::app::ModelFormField;

    let form = match &app.model_form {
        Some(f) => f,
        None => return,
    };

    let title = match &form.mode {
        crate::app::ModelFormMode::Add => " Add Model ",
        crate::app::ModelFormMode::Edit(_) => " Edit Model ",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));

    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // fields
            Constraint::Length(2), // error/help
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let field_area = chunks[0];
    let help_area = chunks[1];
    let footer_area = chunks[2];

    // Build field labels and values
    let field_labels = [
        "Name",
        "Model ID",
        "Provider",
        "API Base",
        "API Key",
        "Max Tokens",
        "Temperature",
        "Thinking Budget",
    ];

    let mut lines: Vec<Line> = Vec::new();

    for (i, label) in field_labels.iter().enumerate() {
        let active_field = ModelFormField::ALL[form.current_field];
        let is_active = active_field == ModelFormField::ALL[i];
        let is_provider = active_field == ModelFormField::Provider;

        // Cursor indicator
        let cursor = if is_active { "▸ " } else { "  " };
        let label_style = if is_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let value = &form.field_buffers[i];
        let display_value = if i == 4 && !is_active {
            // Mask API key when not editing
            "*".repeat(value.len().min(20))
        } else if value.is_empty() {
            "<empty>".to_string()
        } else {
            value.clone()
        };

        let value_style = if value.is_empty() && !is_active {
            Style::default().fg(Color::DarkGray)
        } else if is_active {
            Style::default().fg(Color::White)
        } else if i == 4 {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };

        // Cursor field editing indicator
        let cursor_char = if is_active && !is_provider {
            // Show cursor position within the field
            let pos = form.field_cursors[form.current_field].min(display_value.len());
            format!("{}▌", &display_value[..pos])
        } else {
            display_value.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{cursor}{label:>16}: "), label_style),
            Span::styled(cursor_char, value_style),
        ]));

        // If provider field is active, show hint
        if is_provider && is_active {
            lines.push(Line::from(Span::styled(
                "                    (OpenAI-compatible only)",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let fields_para = Paragraph::new(lines).wrap(Wrap { trim: false });
    Widget::render(&fields_para, field_area, buf);

    // Error or help text
    let mut help_lines: Vec<Line> = Vec::new();
    if let Some(err) = &form.error {
        help_lines.push(Line::from(Span::styled(
            format!("  Error: {err}"),
            Style::default().fg(Color::Red),
        )));
    } else {
        help_lines.push(Line::from(Span::styled(
            "  Tab/Shift+Tab to navigate fields",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let help_para = Paragraph::new(help_lines);
    Widget::render(&help_para, help_area, buf);

    // Footer
    let footer_line = Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Save  "),
        Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Cancel"),
        Span::raw("  "),
        Span::styled("[Tab]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Next Field  "),
        Span::styled("[Del]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" Clear Field"),
    ]);
    let footer_para = Paragraph::new(footer_line);
    Widget::render(&footer_para, footer_area, buf);
}

fn draw_autocomplete(
    ac: &AutocompleteState,
    x: u16,
    y: u16,
    width: u16,
    screen_area: Rect,
    buf: &mut Buffer,
) {
    let max_visible = 8u16;
    let count = ac.matches.len() as u16;
    let visible = count.min(max_visible);
    if visible == 0 {
        return;
    }

    // Clamp to screen bounds
    let available_below = screen_area.height.saturating_sub(y);
    let visible = visible.min(available_below.saturating_sub(1));
    if visible == 0 {
        return;
    }

    let area = Rect {
        x,
        y,
        width: width.min(screen_area.width.saturating_sub(x)),
        height: visible,
    };

    let selected_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    let normal_style = Style::default().fg(Color::White);
    let desc_style = Style::default().fg(Color::DarkGray);

    for (i, (name, desc)) in ac.matches.iter().enumerate().take(visible as usize) {
        let row_y = area.y + i as u16;
        let style = if i == ac.selected { selected_style } else { normal_style };

        // Clear row
        for col in 0..area.width {
            buf[(area.x + col, row_y)].set_char(' ').set_style(style);
        }

        // Draw command name
        let name_chars: Vec<char> = name.chars().collect();
        for (ci, &ch) in name_chars.iter().enumerate() {
            let col = ci as u16;
            if col + 1 >= area.width {
                break;
            }
            buf[(area.x + col, row_y)].set_char(ch).set_style(style);
        }

        // Draw description right-aligned if space permits
        let desc_text = desc.as_str();
        let desc_len = desc_text.chars().count() as u16;
        let name_len = name_chars.len() as u16;
        let min_gap = 2u16;
        if i != ac.selected && name_len + min_gap + desc_len < area.width {
            let desc_start = area.width.saturating_sub(desc_len);
            let style = if i == ac.selected { selected_style } else { desc_style };
            for (ci, ch) in desc_text.chars().enumerate() {
                let col = desc_start + ci as u16;
                if col >= area.width {
                    break;
                }
                buf[(area.x + col, row_y)].set_char(ch).set_style(style);
            }
        }
    }
}