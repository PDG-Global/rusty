// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::app::{AppState, MessageRole};

pub fn draw(app: &AppState, area: Rect, buf: &mut Buffer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Chat area
            Constraint::Length(5), // Input area (2 border + 3 content lines)
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    draw_chat(app, chunks[0], buf);
    draw_input(app, chunks[1], buf);
    draw_status(app, chunks[2], buf);

    // Overlay permission prompt if active
    if app.permission_prompt.is_some() {
        // Dim the entire screen behind the prompt
        let dim_style = Style::default().fg(Color::DarkGray).bg(Color::Black);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_style(dim_style);
            }
        }
        let prompt_area = centered_rect(60, 10, area);
        draw_permission_prompt(app, prompt_area, buf);
    }
}

fn draw_chat(app: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title("rusty");

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
            MessageRole::System => ("! ", Style::default().fg(Color::Yellow)),
        };

        render_content(&msg.content, prefix, base_style, &mut lines, width);
        lines.push(Line::from(""));
    }

    // Thinking text (dimmed, collapsed)
    if app.is_thinking && !app.thinking_text.is_empty() {
        let think_style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        lines.push(Line::from(Span::styled("  \u{1F4AD} thinking...", think_style)));
        // Show last few lines of thinking
        let think_lines: Vec<&str> = app.thinking_text.lines().collect();
        let start = think_lines.len().saturating_sub(5);
        for line in &think_lines[start..] {
            let wrapped = wrap_text(line, width.saturating_sub(4), "    ");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(
                    format!("    {}", wline.trim_start()),
                    think_style,
                )));
            }
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

    // Scroll to bottom
    let visible_height = inner.height as usize;
    let scroll = if lines.len() > visible_height {
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

    for (i, line_text) in msg_lines.iter().enumerate() {
        let line_str = *line_text;

        // Toggle code blocks
        if line_str.starts_with("```") {
            // Flush any pending table
            if !table_buf.is_empty() {
                render_table(&table_buf, base_style, lines);
                table_buf.clear();
            }
            in_code_block = !in_code_block;
            let wrapped = wrap_text(&format!("  {line_str}"), width, "");
            for wline in wrapped {
                lines.push(Line::from(Span::styled(wline, Style::default().fg(Color::DarkGray))));
            }
            continue;
        }

        if in_code_block {
            let wrapped = wrap_text(&format!("  {line_str}"), width, "    ");
            for (j, wline) in wrapped.iter().enumerate() {
                let style = Style::default().fg(Color::Green);
                if j == 0 {
                    lines.push(Line::from(Span::styled(wline.clone(), style)));
                } else {
                    lines.push(Line::from(Span::styled(wline.clone(), style)));
                }
            }
            continue;
        }

        // Detect table rows: line contains | and looks like a table
        if is_table_row(line_str) {
            // Skip separator rows (|---|---|)
            if is_table_separator(line_str) {
                continue;
            }
            let cells = parse_table_row(line_str);
            table_buf.push(cells);
            continue;
        }

        // Flush any pending table before rendering non-table content
        if !table_buf.is_empty() {
            render_table(&table_buf, base_style, lines);
            table_buf.clear();
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
        render_table(&table_buf, base_style, lines);
    }
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
    // Looks like |---|---| or | --- | --- |
    trimmed.starts_with('|')
        && trimmed
            .chars()
            .all(|c| "|- \t:".contains(c))
        && trimmed.contains('-')
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

fn render_table(rows: &[Vec<String>], base_style: Style, lines: &mut Vec<Line>) {
    if rows.is_empty() {
        return;
    }

    // Calculate column widths
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths = vec![0usize; num_cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(cell.len());
            }
        }
    }
    // Clamp widths to avoid overflow
    let max_width = 80;
    let total: usize = col_widths.iter().sum();
    if total + num_cols * 3 > max_width {
        let scale = (max_width - num_cols * 3) as f64 / total.max(1) as f64;
        for w in &mut col_widths {
            *w = ((*w as f64 * scale) as usize).max(3);
        }
    }

    // Build top border
    let mut top = String::from("  \u{250C}"); // ┌
    for (i, w) in col_widths.iter().enumerate() {
        top.push_str(&"\u{2500}".repeat(w + 2)); // ─
        if i < num_cols - 1 {
            top.push('\u{252C}'); // ┬
        }
    }
    top.push('\u{2510}'); // ┐
    lines.push(Line::from(Span::styled(
        top,
        Style::default().fg(Color::DarkGray),
    )));

    // Render rows
    for (row_idx, row) in rows.iter().enumerate() {
        let mut spans = vec![Span::styled(
            "  \u{2502}".to_string(), // │
            Style::default().fg(Color::DarkGray),
        )];
        for (col, cell) in row.iter().enumerate() {
            let w = col_widths.get(col).copied().unwrap_or(10);
            let truncated: String = cell.chars().take(w).collect();
            let pad = w.saturating_sub(truncated.len());
            let padded = format!(" {}{} ", truncated, " ".repeat(pad));
            spans.push(Span::styled(
                padded,
                base_style,
            ));
            spans.push(Span::styled(" ".to_string(), Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                "\u{2502}".to_string(), // │
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));

        // Separator after header row (first row)
        if row_idx == 0 {
            let mut sep = String::from("  \u{251C}"); // ├
            for (i, w) in col_widths.iter().enumerate() {
                sep.push_str(&"\u{2500}".repeat(w + 2));
                if i < num_cols - 1 {
                    sep.push('\u{253C}'); // ┼
                }
            }
            sep.push('\u{2524}'); // ┤
            lines.push(Line::from(Span::styled(
                sep,
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Bottom border
    let mut bot = String::from("  \u{2514}"); // └
    for (i, w) in col_widths.iter().enumerate() {
        bot.push_str(&"\u{2500}".repeat(w + 2));
        if i < num_cols - 1 {
            bot.push('\u{2534}'); // ┴
        }
    }
    bot.push('\u{2518}'); // ┘
    lines.push(Line::from(Span::styled(
        bot,
        Style::default().fg(Color::DarkGray),
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
                    Style::default().fg(Color::Green).bg(Color::DarkGray),
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
        if i + delim_len <= chars.len() && chars[i..i + delim_len] == *delim_chars {
            if i > start {
                return Some(i);
            }
        }
    }
    None
}

fn draw_input(app: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.is_streaming {
            Color::DarkGray
        } else {
            Color::Green
        }))
        .title(if app.is_streaming {
            " processing... "
        } else {
            " input "
        });

    let inner = block.inner(area);
    block.render(area, buf);

    let (input_text, style) = if app.is_streaming && app.input.is_empty() {
        let status = if let Some(tool) = app.pending_tools.last() {
            let label = crate::app::friendly_tool_name(&tool.name);
            format!("{label}...")
        } else {
            "waiting for response...".to_string()
        };
        (status, Style::default().fg(Color::DarkGray))
    } else if app.input.is_empty() {
        (
            "Type a message...".to_string(),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (app.input.clone(), Style::default().fg(Color::White))
    };

    // Calculate which row the cursor is on to scroll to it
    let width = inner.width.max(1) as usize;
    let cursor_row = app.cursor_pos / width;
    let visible_rows = inner.height as usize;
    let scroll = if cursor_row + 1 >= visible_rows {
        cursor_row + 1 - visible_rows
    } else {
        0
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(input_text, style)))
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    Widget::render(&paragraph, inner, buf);
}

fn draw_status(app: &AppState, area: Rect, buf: &mut Buffer) {
    let status_style = Style::default().fg(Color::DarkGray);
    let status_text = format!(
        " {} | in: {} out: {} | {}",
        app.status.model,
        app.status.input_tokens,
        app.status.output_tokens,
        if app.is_streaming {
            "streaming"
        } else {
            "ready"
        },
    );
    let paragraph = Paragraph::new(Line::from(Span::styled(status_text, status_style)));
    Widget::render(&paragraph, area, buf);
}

/// Create a centered rectangle with the given percentage width and fixed height.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_height = height.min(area.height);
    let popup_width = (area.width * percent_x / 100).min(area.width);
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(area.x + x, area.y + y, popup_width, popup_height)
}

fn draw_permission_prompt(app: &AppState, area: Rect, buf: &mut Buffer) {
    use rusty_core::permissions::build_tool_description;

    let prompt = match &app.permission_prompt {
        Some(p) => p,
        None => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Permission Required ");

    let inner = block.inner(area);
    block.render(area, buf);

    let desc = build_tool_description(&prompt.request.tool_name, &prompt.request.raw_input);
    let tool_label = &prompt.request.tool_name;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Tool: ", Style::default().fg(Color::DarkGray)),
        Span::styled(tool_label.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Command: ", Style::default().fg(Color::DarkGray)),
        Span::styled(desc, Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Allow once   "),
        Span::styled("[n]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw(" Deny"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("[a]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Allow session "),
        Span::styled("[d]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::raw(" Allow always"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("[Esc]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::raw(" Cancel"),
    ]));

    let paragraph = Paragraph::new(lines);
    Widget::render(&paragraph, inner, buf);
}
