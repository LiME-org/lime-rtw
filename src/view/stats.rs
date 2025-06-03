use serde_json::Value;
use std::collections::HashMap;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::data::ProcessInfo;

// Manages the stats view showing separators and their details
#[derive(Default)]
pub struct StatsView {
    separator_state: ListState,
    selected_separator_idx: Option<usize>,
    detail_scroll_offset: usize,
    horizontal_offset: usize,
}

impl StatsView {
    // Creates a new stats view with default separator selection
    pub fn new() -> Self {
        let mut separator_state = ListState::default();
        separator_state.select(Some(0));

        Self {
            separator_state,
            selected_separator_idx: Some(0),
            detail_scroll_offset: 0,
            horizontal_offset: 0,
        }
    }

    // Renders the stats view in split mode (separators + details)
    pub fn draw<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        area: Rect,
        process: &ProcessInfo,
        custom_block: Option<Block<'_>>,
    ) {
        let task_id = process.task_id;
        let comm = process
            .info
            .comm
            .clone()
            .unwrap_or_else(|| String::from("-"));
        let title = format!("Models for {} ({})", comm, task_id);

        if let Some(models) = &process.models {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
                .split(area);

            let separator_data = self.extract_separator_data(models);

            let mut separator_names_with_count: Vec<(String, usize)> = separator_data
                .keys()
                .map(|name| (name.clone(), self.count_separator_occurrences(models, name)))
                .collect();

            separator_names_with_count.sort_by(|a, b| {
                let count_cmp = b.1.cmp(&a.1);
                if count_cmp == std::cmp::Ordering::Equal {
                    a.0.cmp(&b.0)
                } else {
                    count_cmp
                }
            });

            let separator_names: Vec<String> = separator_names_with_count
                .iter()
                .map(|(name, _)| name.clone())
                .collect();

            self.validate_selected_index(&separator_names);

            let items: Vec<ListItem> = separator_names_with_count
                .iter()
                .map(|(separator, count)| {
                    let model_info = &separator_data[separator];
                    let has_period = model_info.period.is_some();
                    let style = if has_period {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default()
                    };

                    let mut spans = vec![
                        Span::styled(separator.clone(), style),
                        Span::raw(" "),
                        Span::styled(
                            format!("[{} occurrences]", count),
                            Style::default().fg(Color::Yellow),
                        ),
                    ];

                    if has_period {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled("[P]", Style::default().fg(Color::Yellow)));
                    }

                    ListItem::new(Line::from(spans))
                })
                .collect();

            let model_block =
                custom_block.unwrap_or_else(|| Block::default().title(title).borders(Borders::ALL));

            let separator_list = List::new(items)
                .block(
                    Block::default()
                        .title("Separators (Sorted by Count)")
                        .borders(Borders::NONE),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol(">> ");

            f.render_stateful_widget(separator_list, chunks[0], &mut self.separator_state);

            f.render_widget(model_block, area);

            if let Some(idx) = self.selected_separator_idx {
                if idx < separator_names.len() {
                    let separator_name = &separator_names[idx];
                    let model_info = &separator_data[separator_name];

                    let model_text = self.format_model_info(separator_name, model_info);

                    let details_block = Block::default()
                        .title(format!("Details for '{}'", separator_name))
                        .borders(Borders::NONE);

                    let details = Paragraph::new(model_text)
                        .block(details_block)
                        .wrap(Wrap { trim: false });

                    f.render_widget(details, chunks[1]);
                }
            }
        } else {
            let title_block =
                custom_block.unwrap_or_else(|| Block::default().title(title).borders(Borders::ALL));

            let paragraph = Paragraph::new("No model data available for this process")
                .block(title_block)
                .wrap(Wrap { trim: false });

            f.render_widget(paragraph, area);
        }
    }

    // Validates and adjusts the selected separator index
    fn validate_selected_index(&mut self, separator_names: &[String]) {
        if separator_names.is_empty() {
            self.selected_separator_idx = None;
            self.separator_state.select(None);
            return;
        }

        if self.selected_separator_idx.is_none() {
            self.selected_separator_idx = Some(0);
            self.separator_state.select(Some(0));
            return;
        }

        if let Some(idx) = self.selected_separator_idx {
            if idx >= separator_names.len() {
                self.selected_separator_idx = Some(0);
                self.separator_state.select(Some(0));
            }
        }
    }

    // Navigate to the next separator
    pub fn next_separator(&mut self) {
        if let Some(current_idx) = self.selected_separator_idx {
            self.selected_separator_idx = Some(current_idx.saturating_add(1));
            self.separator_state.select(self.selected_separator_idx);
        }
    }

    // Navigate to the previous separator
    pub fn previous_separator(&mut self) {
        if let Some(current_idx) = self.selected_separator_idx {
            self.selected_separator_idx = Some(current_idx.saturating_sub(1));
            self.separator_state.select(self.selected_separator_idx);
        }
    }

    // Returns true if a separator is currently selected
    pub fn has_separator_selected(&self) -> bool {
        self.selected_separator_idx.is_some()
    }

    // Scroll down in the detail view
    pub fn scroll_detail_view_down(&mut self) {
        self.detail_scroll_offset = self.detail_scroll_offset.saturating_add(5);
    }

    // Scroll up in the detail view
    pub fn scroll_detail_view_up(&mut self) {
        self.detail_scroll_offset = self.detail_scroll_offset.saturating_sub(5);
    }

    // Reset detail view scroll position to top
    pub fn reset_detail_scroll(&mut self) {
        self.detail_scroll_offset = 0;
    }

    // Scroll left in the detail view
    pub fn scroll_left(&mut self, _area_width: u16) {
        self.horizontal_offset = self.horizontal_offset.saturating_sub(10);
    }

    // Scroll right in the detail view
    pub fn scroll_right(&mut self, _area_width: u16) {
        self.horizontal_offset += 10;
    }

    // Reset horizontal offset
    pub fn handle_escape(&mut self) {
        self.horizontal_offset = 0;
        self.reset_detail_scroll();
    }

    // Renders the stats view in split layout with separate areas for list and details
    pub fn draw_split<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        list_area: Rect,
        details_area: Rect,
        process: &ProcessInfo,
        list_block: Option<Block<'_>>,
        details_block: Option<Block<'_>>,
    ) {
        if let Some(models) = &process.models {
            let horizontal_offset = self.horizontal_offset;
            let selected_separator_idx = self.selected_separator_idx;

            let separator_data = self.extract_separator_data(models);

            let mut separator_names_with_count: Vec<(String, usize)> = separator_data
                .keys()
                .map(|name| (name.clone(), self.count_separator_occurrences(models, name)))
                .collect();

            separator_names_with_count.sort_by(|a, b| {
                let count_cmp = b.1.cmp(&a.1);
                if count_cmp == std::cmp::Ordering::Equal {
                    a.0.cmp(&b.0)
                } else {
                    count_cmp
                }
            });

            let separator_names: Vec<String> = separator_names_with_count
                .iter()
                .map(|(name, _)| name.clone())
                .collect();

            self.validate_selected_index(&separator_names);

            let items: Vec<ListItem> = separator_names_with_count
                .iter()
                .map(|(separator, count)| {
                    let model_info = &separator_data[separator];
                    let has_period = model_info.period.is_some();
                    let style = if has_period {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default()
                    };

                    let mut spans = vec![
                        Span::styled(separator.clone(), style),
                        Span::raw(" "),
                        Span::styled(
                            format!("[{} occurrences]", count),
                            Style::default().fg(Color::Yellow),
                        ),
                    ];

                    if has_period {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled("[P]", Style::default().fg(Color::Yellow)));
                    }

                    let scrolled_spans =
                        Self::apply_horizontal_scroll(spans, list_area.width, horizontal_offset);
                    ListItem::new(Line::from(scrolled_spans))
                })
                .collect();

            let list_block_final = list_block.unwrap_or_else(|| {
                Block::default()
                    .title("Separators (Sorted by Count)")
                    .borders(Borders::ALL)
            });

            let separator_list = List::new(items)
                .block(list_block_final)
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol(">> ");

            let details_content = if let Some(idx) = selected_separator_idx {
                if idx < separator_names.len() {
                    let separator_name = &separator_names[idx];
                    let model_info = &separator_data[separator_name];
                    Some((
                        separator_name.clone(),
                        self.format_model_info(separator_name, model_info),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            f.render_stateful_widget(separator_list, list_area, &mut self.separator_state);

            if let Some((separator_name, model_text)) = details_content {
                let final_details_block = details_block.unwrap_or_else(|| {
                    Block::default()
                        .title(format!("Details for '{}'", separator_name))
                        .borders(Borders::ALL)
                });

                let details = Paragraph::new(model_text)
                    .block(final_details_block)
                    .wrap(Wrap { trim: false });

                f.render_widget(details, details_area);
            } else if selected_separator_idx.is_some() {
                let final_details_block = details_block.unwrap_or_else(|| {
                    Block::default()
                        .title("Separator Details")
                        .borders(Borders::ALL)
                });

                let empty_details =
                    Paragraph::new("Select a separator to view details").block(final_details_block);

                f.render_widget(empty_details, details_area);
            } else {
                let final_details_block = details_block.unwrap_or_else(|| {
                    Block::default()
                        .title("Separator Details")
                        .borders(Borders::ALL)
                });

                let empty_details =
                    Paragraph::new("No separators available").block(final_details_block);

                f.render_widget(empty_details, details_area);
            }
        } else {
            let empty_list_block = list_block
                .unwrap_or_else(|| Block::default().title("Separators").borders(Borders::ALL));

            let empty_details_block = details_block.unwrap_or_else(|| {
                Block::default()
                    .title("Separator Details")
                    .borders(Borders::ALL)
            });

            let no_data_msg =
                Paragraph::new("No model data available for this process").block(empty_list_block);

            let empty_details =
                Paragraph::new("No model data available").block(empty_details_block);

            f.render_widget(no_data_msg, list_area);
            f.render_widget(empty_details, details_area);
        }
    }

    fn extract_separator_data(&self, model_value: &Value) -> HashMap<String, ModelInfo> {
        let mut result = HashMap::new();

        if model_value.is_object() {
            if let Some(separators) = model_value.get("by_job_separator") {
                if let Some(seps_array) = separators.as_array() {
                    for sep in seps_array {
                        if let Some(separator_obj) = sep.get("separator") {
                            let sep_type = separator_obj
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("unnamed");

                            let mut sep_details = String::new();
                            if let Some(obj) = separator_obj.as_object() {
                                if let Some(clock_id) = obj.get("clock_id").and_then(|c| c.as_str())
                                {
                                    let abs_str = if obj.get("abs_time").is_some() {
                                        ", absolute"
                                    } else {
                                        ""
                                    };

                                    sep_details = format!("({}{})", clock_id, abs_str);
                                }
                            }

                            let separator_name = if sep_details.is_empty() {
                                sep_type.to_string()
                            } else {
                                format!("{} {}", sep_type, sep_details)
                            };

                            let mut model_info = ModelInfo {
                                raw_data: sep.clone(),
                                ..Default::default()
                            };

                            if let Some(count) = sep.get("count").and_then(|c| c.as_u64()) {
                                model_info.count = Some(count);
                            }

                            if let Some(arrival_models) = sep.get("arrival_models") {
                                if let Some(models_array) = arrival_models.as_array() {
                                    for model in models_array {
                                        if let Some("periodic") =
                                            model.get("model").and_then(|t| t.as_str())
                                        {
                                            model_info.period =
                                                model.get("period").and_then(|p| p.as_u64());
                                            model_info.jitter =
                                                model.get("max_jitter").and_then(|j| j.as_u64());
                                        }
                                    }
                                }
                            }

                            result.insert(separator_name, model_info);
                        } else {
                            let key = format!("Item_{}", result.len());
                            let model_info = ModelInfo {
                                raw_data: sep.clone(),
                                ..Default::default()
                            };
                            result.insert(key, model_info);
                        }
                    }
                }
            } else {
                let model_info = ModelInfo {
                    raw_data: model_value.clone(),
                    ..Default::default()
                };
                result.insert("Main".to_string(), model_info);
            }

            return result;
        }

        if let Some(items) = model_value.as_array() {
            for item in items {
                if !item.is_object() {
                    continue;
                }

                let separator_name = item
                    .get("separator")
                    .and_then(|s| s.as_str())
                    .or_else(|| item.get("name").and_then(|n| n.as_str()))
                    .or_else(|| item.get("id").and_then(|i| i.as_str()))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Item_{}", result.len()));

                let mut model_info = ModelInfo {
                    raw_data: item.clone(),
                    ..Default::default()
                };

                if let Some(models) = item.get("arrival_models") {
                    if let Some(models_array) = models.as_array() {
                        for model in models_array {
                            if let Some("periodic") = model.get("type").and_then(|t| t.as_str()) {
                                model_info.period = model.get("period").and_then(|p| p.as_u64());
                                model_info.jitter = model.get("jitter").and_then(|j| j.as_u64());
                            }
                        }
                    }
                } else if let Some(period) = item.get("period").and_then(|p| p.as_u64()) {
                    model_info.period = Some(period);
                }

                result.insert(separator_name, model_info);
            }
        }

        result
    }

    fn count_separator_occurrences(&self, model_value: &Value, separator_name: &str) -> usize {
        if separator_name.starts_with("Item_") || separator_name == "Main" {
            return 1;
        }

        if model_value.is_object() {
            if let Some(separators) = model_value.get("by_job_separator") {
                if let Some(seps_array) = separators.as_array() {
                    for sep in seps_array {
                        if let Some(separator_obj) = sep.get("separator") {
                            let sep_type = separator_obj
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("");

                            if sep_type.is_empty() {
                                continue;
                            }

                            let mut sep_details = String::new();
                            if let Some(obj) = separator_obj.as_object() {
                                if let Some(clock_id) = obj.get("clock_id").and_then(|c| c.as_str())
                                {
                                    let abs_str = if obj.get("abs_time").is_some() {
                                        ", absolute"
                                    } else {
                                        ""
                                    };

                                    sep_details = format!("({}{})", clock_id, abs_str);
                                }
                            }

                            let name = if sep_details.is_empty() {
                                sep_type.to_string()
                            } else {
                                format!("{} {}", sep_type, sep_details)
                            };

                            if name == separator_name {
                                return sep
                                    .get("count")
                                    .and_then(|c| c.as_u64())
                                    .map(|c| c as usize)
                                    .unwrap_or(1);
                            }
                        }
                    }
                }
            }
            return 1;
        }

        let mut count = 0;
        if let Some(items) = model_value.as_array() {
            for item in items {
                let name = item
                    .get("separator")
                    .and_then(|s| s.as_str())
                    .or_else(|| item.get("name").and_then(|n| n.as_str()))
                    .or_else(|| item.get("id").and_then(|i| i.as_str()));

                if let Some(n) = name {
                    if n == separator_name {
                        count += 1;
                    }
                }
            }
        }

        count.max(1)
    }

    fn format_model_info(&self, separator_name: &str, model_info: &ModelInfo) -> String {
        let mut details = Vec::new();

        details.push(format!("Separator: {}", separator_name));
        details.push(String::new());

        if let Some(count) = model_info.count {
            details.push(format!("Count: {}", count));
            details.push(String::new());
        }

        if let Some(arrival_models) = model_info
            .raw_data
            .get("arrival_models")
            .and_then(|v| v.as_array())
        {
            details.push(String::from("Arrival Models:"));
            details.push("─".repeat(40));

            for (idx, model) in arrival_models.iter().enumerate() {
                if let Some(model_type) = model.get("model").and_then(|t| t.as_str()) {
                    let display_type = match model_type {
                        "arrival_curve" => "arrival curve",
                        _ => model_type,
                    };
                    details.push(format!("Model #{}: {}", idx + 1, display_type));

                    match model_type {
                        "periodic" => {
                            if let Some(period) = model.get("period").and_then(|p| p.as_u64()) {
                                details.push(format!("  Period: {} ns", period));
                            }
                            if let Some(jitter) = model.get("max_jitter").and_then(|j| j.as_u64()) {
                                details.push(format!("  Jitter: {} ns", jitter));
                            }
                            if let Some(offset) = model.get("offset").and_then(|o| o.as_u64()) {
                                details.push(format!("  Offset: {} ns", offset));
                            }
                            if let Some(on) = model.get("on").and_then(|o| o.as_str()) {
                                details.push(format!("  On: {}", on));
                            }
                        }
                        "sporadic" => {
                            if let Some(mit) = model.get("mit").and_then(|m| m.as_u64()) {
                                details.push(format!("  Minimum Inter-arrival Time: {} ns", mit));
                            }
                        }
                        "arrival_curve" => {
                            let dmin_count = model
                                .get("dmins")
                                .and_then(|d| d.as_array())
                                .map_or(0, |arr| arr.len());
                            details
                                .push(format!("  Points: {} (see tables for details)", dmin_count));
                        }
                        _ => {
                            if let Some(obj) = model.as_object() {
                                for (k, v) in obj {
                                    if k != "model" {
                                        let value_str = match v {
                                            Value::Null => "null".to_string(),
                                            Value::Bool(b) => {
                                                (if *b { "true" } else { "false" }).to_string()
                                            }
                                            Value::Number(n) => {
                                                if n.is_u64() {
                                                    format!("{}", n.as_u64().unwrap())
                                                } else if n.is_i64() {
                                                    format!("{}", n.as_i64().unwrap())
                                                } else {
                                                    format!("{:.2}", n.as_f64().unwrap())
                                                }
                                            }
                                            Value::String(s) => s.to_string(),
                                            Value::Array(_) => "[array]".to_string(),
                                            Value::Object(_) => "{object}".to_string(),
                                        };
                                        details.push(format!("  {}: {}", k, value_str));
                                    }
                                }
                            }
                        }
                    }
                    details.push(String::new());
                }
            }

            if let Some(dss) = model_info
                .raw_data
                .get("dynamic_self_suspension")
                .and_then(|v| v.as_u64())
                .filter(|&val| val > 0)
            {
                details.push(format!("Dynamic Self-Suspension: {} ns", dss));
                details.push(String::new());
            }
        } else if let Some(period) = model_info.period {
            details.push(String::from("Periodic Model:"));
            details.push(format!("  Period: {} ns", period));

            if let Some(jitter) = model_info.jitter {
                details.push(format!("  Jitter: {} ns", jitter));
            }

            details.push(String::new());
        }

        details.join("\n")
    }

    pub fn draw_detail_fullscreen<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        area: Rect,
        process: &ProcessInfo,
    ) {
        if let Some(models) = &process.models {
            let separator_data = self.extract_separator_data(models);

            let mut separator_names_with_count: Vec<(String, usize)> = separator_data
                .keys()
                .map(|name| (name.clone(), self.count_separator_occurrences(models, name)))
                .collect();

            separator_names_with_count.sort_by(|a, b| {
                let count_cmp = b.1.cmp(&a.1);
                if count_cmp == std::cmp::Ordering::Equal {
                    a.0.cmp(&b.0)
                } else {
                    count_cmp
                }
            });

            let separator_names: Vec<String> = separator_names_with_count
                .iter()
                .map(|(name, _)| name.clone())
                .collect();

            if let Some(idx) = self.selected_separator_idx {
                if idx < separator_names.len() {
                    let separator_name = &separator_names[idx];
                    let model_info = &separator_data[separator_name];

                    let block = Block::default()
                        .title(format!(
                            "Detailed View: {} (Scroll: Up/Down)",
                            separator_name
                        ))
                        .borders(Borders::ALL);

                    let inner_area = block.inner(area);
                    f.render_widget(block, area);

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .margin(1)
                        .constraints(
                            [
                                Constraint::Length(3),
                                Constraint::Length(1),
                                Constraint::Min(10),
                            ]
                            .as_ref(),
                        )
                        .split(inner_area);

                    self.render_separator_summary(f, chunks[0], separator_name, model_info);
                    self.render_separator_details(f, chunks[2], model_info);

                    return;
                }
            }

            let block = Block::default()
                .title("No separator selected")
                .borders(Borders::ALL);

            let message = Paragraph::new("Select a separator from the list to view details")
                .block(block)
                .wrap(Wrap { trim: false });

            f.render_widget(message, area);
        } else {
            let block = Block::default()
                .title("No model data")
                .borders(Borders::ALL);

            let message = Paragraph::new("No model data available for this process")
                .block(block)
                .wrap(Wrap { trim: false });

            f.render_widget(message, area);
        }
    }

    fn render_separator_summary<B: Backend>(
        &self,
        f: &mut Frame<B>,
        area: Rect,
        separator_name: &str,
        model_info: &ModelInfo,
    ) {
        let mut summary_text = Vec::new();

        let count_str = if let Some(count) = model_info.count {
            format!("Count: {}", count)
        } else {
            String::from("Count: Unknown")
        };

        let period_str = if let Some(period) = model_info.period {
            if let Some(jitter) = model_info.jitter {
                format!("Period: {} ns, Jitter: {} ns", period, jitter)
            } else {
                format!("Period: {} ns", period)
            }
        } else {
            String::from("No periodic model")
        };

        summary_text.push(Line::from(vec![
            Span::styled(
                "Separator: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                separator_name.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(count_str, Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled(period_str, Style::default().fg(Color::Cyan)),
        ]));

        let summary = Paragraph::new(summary_text)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: false });

        f.render_widget(summary, area);
    }

    fn render_separator_details<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        area: Rect,
        model_info: &ModelInfo,
    ) {
        let mut detail_lines = Vec::new();

        if let Some(arrival_models) = model_info
            .raw_data
            .get("arrival_models")
            .and_then(|v| v.as_array())
        {
            let mut has_models = false;

            for model in arrival_models {
                if let Some(model_type) = model.get("model").and_then(|t| t.as_str()) {
                    if model_type == "periodic" || model_type == "sporadic" {
                        if !has_models {
                            detail_lines.push(Line::from(vec![Span::styled(
                                "Available Models:",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )]));
                            has_models = true;
                        }

                        detail_lines.push(Line::from(vec![Span::styled(
                            format!("► {}", model_type),
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )]));

                        match model_type {
                            "periodic" => {
                                let period = model.get("period").and_then(|p| p.as_u64());

                                if let Some(p) = period {
                                    detail_lines.push(Line::from(vec![
                                        Span::raw("  "),
                                        Span::styled("Period:", Style::default().fg(Color::Blue)),
                                        Span::raw(" "),
                                        Span::styled(format!("{} ns", p), Style::default()),
                                    ]));
                                }

                                let jitter = model
                                    .get("max_jitter")
                                    .or_else(|| model.get("jitter"))
                                    .and_then(|j| j.as_u64());

                                if let Some(j) = jitter {
                                    detail_lines.push(Line::from(vec![
                                        Span::raw("  "),
                                        Span::styled("Jitter:", Style::default().fg(Color::Blue)),
                                        Span::raw(" "),
                                        Span::styled(format!("{} ns", j), Style::default()),
                                    ]));
                                }

                                if let Some(offset) = model.get("offset").and_then(|o| o.as_u64()) {
                                    detail_lines.push(Line::from(vec![
                                        Span::raw("  "),
                                        Span::styled("Offset:", Style::default().fg(Color::Blue)),
                                        Span::raw(" "),
                                        Span::styled(format!("{} ns", offset), Style::default()),
                                    ]));
                                }

                                if let Some(on) = model.get("on").and_then(|o| o.as_str()) {
                                    detail_lines.push(Line::from(vec![
                                        Span::raw("  "),
                                        Span::styled("On:", Style::default().fg(Color::Blue)),
                                        Span::raw(" "),
                                        Span::styled(on.to_string(), Style::default()),
                                    ]));
                                }
                            }
                            "sporadic" => {
                                if let Some(mit) = model.get("mit").and_then(|m| m.as_u64()) {
                                    detail_lines.push(Line::from(vec![
                                        Span::raw("  "),
                                        Span::styled(
                                            "Minimum Inter-Arrival Time:",
                                            Style::default().fg(Color::Blue),
                                        ),
                                        Span::raw(" "),
                                        Span::styled(format!("{} ns", mit), Style::default()),
                                    ]));
                                }
                            }
                            _ => {}
                        }

                        detail_lines.push(Line::from(""));
                    }
                }
            }

            if has_models {
                detail_lines.push(Line::from("─".repeat(area.width as usize - 2)));
                detail_lines.push(Line::from(""));
            }
        }

        let has_suspension_data = model_info.raw_data.get("dynamic_self_suspension").is_some()
            || model_info
                .raw_data
                .get("segmented_self_suspensions")
                .is_some();

        if has_suspension_data {
            detail_lines.push(Line::from(vec![Span::styled(
                "Self-Suspension Information:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
            detail_lines.push(Line::from(""));

            if let Some(dss) = model_info.raw_data.get("dynamic_self_suspension") {
                detail_lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("Dynamic Self-Suspension:", Style::default().fg(Color::Blue)),
                    Span::raw(" "),
                    Span::styled(format!("{} ns", dss), Style::default()),
                ]));
                detail_lines.push(Line::from(""));
            }

            if let Some(sss) = model_info.raw_data.get("segmented_self_suspensions") {
                detail_lines.push(Line::from(vec![Span::styled(
                    "  Segmented Self-Suspensions:",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )]));
                detail_lines.push(Line::from("  ────────────────────────"));

                if let Some(sss_obj) = sss.as_object() {
                    let mut segment_keys: Vec<&String> = sss_obj.keys().collect();
                    segment_keys.sort_by(|a, b| {
                        a.parse::<u64>()
                            .unwrap_or(0)
                            .cmp(&b.parse::<u64>().unwrap_or(0))
                    });

                    for (vec_idx, segment_key) in segment_keys.iter().enumerate() {
                        detail_lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                format!(
                                    "Segment Vector #{} (length: {}):",
                                    vec_idx + 1,
                                    segment_key
                                ),
                                Style::default().fg(Color::Green),
                            ),
                        ]));

                        if let Some(segment_arr) =
                            sss_obj.get(*segment_key).and_then(|s| s.as_array())
                        {
                            for (idx, segment_data) in segment_arr.iter().enumerate() {
                                if let Some(data_arr) = segment_data.as_array() {
                                    if data_arr.len() >= 2 {
                                        let suspension_time = data_arr[0].as_u64().unwrap_or(0);
                                        let execution_time = data_arr[1].as_u64().unwrap_or(0);

                                        detail_lines.push(Line::from(vec![
                                            Span::raw("    "),
                                            Span::styled(
                                                format!("#{}: ", idx + 1),
                                                Style::default().fg(Color::Yellow),
                                            ),
                                            Span::raw("Suspension: "),
                                            Span::styled(
                                                format!("{} ns", suspension_time),
                                                Style::default().fg(Color::Red),
                                            ),
                                            Span::raw(", Execution: "),
                                            Span::styled(
                                                format!("{} ns", execution_time),
                                                Style::default().fg(Color::Green),
                                            ),
                                        ]));
                                    } else {
                                        detail_lines.push(Line::from(vec![
                                            Span::raw("    "),
                                            Span::styled(
                                                format!("#{}: ", idx + 1),
                                                Style::default().fg(Color::Yellow),
                                            ),
                                            Span::raw("Incomplete data"),
                                        ]));
                                    }
                                }
                            }
                        }
                    }
                }

                detail_lines.push(Line::from(""));
            }
        }

        let mut wcet_table_lines = Vec::new();
        let mut arrival_curve_table_lines = Vec::new();

        self.extract_table_data(
            &model_info.raw_data,
            &mut wcet_table_lines,
            &mut arrival_curve_table_lines,
            area.width,
        );

        if !wcet_table_lines.is_empty() || !arrival_curve_table_lines.is_empty() {
            detail_lines.push(Line::from(""));
            detail_lines.push(Line::from("─".repeat(area.width as usize - 2)));
            detail_lines.push(Line::from(""));
            detail_lines.push(Line::from(vec![Span::styled(
                "Tables:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
            detail_lines.push(Line::from(""));
        }

        if !wcet_table_lines.is_empty() {
            for line in wcet_table_lines {
                detail_lines.push(line);
            }
            detail_lines.push(Line::from(""));
        }

        if !arrival_curve_table_lines.is_empty() {
            for line in arrival_curve_table_lines {
                detail_lines.push(line);
            }
            detail_lines.push(Line::from(""));
        }

        let total_lines = detail_lines.len();
        let visible_height = area.height as usize;

        if self.detail_scroll_offset > total_lines.saturating_sub(visible_height) {
            self.detail_scroll_offset = total_lines.saturating_sub(visible_height);
        }

        let start_idx = self.detail_scroll_offset.min(total_lines.saturating_sub(1));
        let end_idx = (start_idx + visible_height).min(total_lines);
        let visible_lines = &detail_lines[start_idx..end_idx];

        let details = Paragraph::new(visible_lines.to_vec())
            .block(
                Block::default()
                    .title("Details".to_string())
                    .borders(Borders::NONE),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(details, area);
    }

    fn extract_table_data(
        &self,
        model_data: &Value,
        wcet_table_lines: &mut Vec<Line<'_>>,
        arrival_curve_table_lines: &mut Vec<Line<'_>>,
        width: u16,
    ) {
        self.extract_wcet_table(model_data, wcet_table_lines, width);
        self.extract_arrival_curve_table(model_data, arrival_curve_table_lines, width);
    }

    fn extract_wcet_table(
        &self,
        model_data: &Value,
        wcet_table_lines: &mut Vec<Line<'_>>,
        width: u16,
    ) {
        if let Some(wcets) = model_data.get("wcet_n").and_then(|w| w.as_array()) {
            self.add_wcet_table_header(wcet_table_lines, width, "WCET_n Table:");
            self.add_wcet_table_content(wcet_table_lines, wcets, width);
            return;
        }

        if let Some(models) = model_data.get("arrival_models") {
            if let Some(models_array) = models.as_array() {
                for model in models_array {
                    if let Some(wcets) = model.get("WCET_n").and_then(|w| w.as_array()) {
                        let model_type = model
                            .get("model")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown");

                        self.add_wcet_table_header(
                            wcet_table_lines,
                            width,
                            &format!("WCET_n Table ({})", model_type),
                        );
                        self.add_wcet_table_content(wcet_table_lines, wcets, width);
                        break;
                    }
                }
            }
        }
    }

    fn add_wcet_table_header(&self, lines: &mut Vec<Line<'_>>, width: u16, title: &str) {
        lines.push(Line::from(vec![
            Span::styled("⭐ ".to_string(), Style::default().fg(Color::Yellow)),
            Span::styled(
                title.to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Worst-Case Execution Time for n jobs",
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("  N", Style::default().fg(Color::Yellow)),
            Span::raw(" | "),
            Span::styled(" WCET (ns)", Style::default().fg(Color::Blue)),
        ]));

        lines.push(Line::from(format!(
            "  {}",
            "─".repeat((width).saturating_sub(4) as usize)
        )));
    }

    fn add_wcet_table_content(&self, lines: &mut Vec<Line<'_>>, wcets: &[Value], _width: u16) {
        let wcet_values: Vec<u64> = wcets.iter().filter_map(|v| v.as_u64()).collect();

        for (idx, &wcet) in wcet_values.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:3}", idx + 1), Style::default().fg(Color::Yellow)),
                Span::raw(" | "),
                Span::styled(format!("{:10}", wcet), Style::default().fg(Color::Blue)),
            ]));
        }
    }

    fn extract_arrival_curve_table(
        &self,
        model_data: &Value,
        arrival_curve_table_lines: &mut Vec<Line<'_>>,
        width: u16,
    ) {
        if let Some(models) = model_data.get("arrival_models") {
            if let Some(models_array) = models.as_array() {
                for model in models_array {
                    let model_type = model
                        .get("model")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");

                    if model_type == "arrival_curve" {
                        let dmins: Vec<u64> = model
                            .get("dmins")
                            .and_then(|dm| dm.as_array())
                            .map_or(Vec::new(), |arr| {
                                arr.iter().filter_map(|v| v.as_u64()).collect()
                            });

                        let dmaxs: Vec<u64> = model
                            .get("dmaxs")
                            .and_then(|dm| dm.as_array())
                            .map_or(Vec::new(), |arr| {
                                arr.iter().filter_map(|v| v.as_u64()).collect()
                            });

                        if !dmins.is_empty() && !dmaxs.is_empty() {
                            self.add_arrival_curve_table_content(
                                arrival_curve_table_lines,
                                &dmins,
                                &dmaxs,
                                width,
                            );
                        }
                        break;
                    }
                }
            }
        }
    }

    fn add_arrival_curve_table_content(
        &self,
        lines: &mut Vec<Line<'_>>,
        dmins: &[u64],
        dmaxs: &[u64],
        width: u16,
    ) {
        lines.push(Line::from(vec![
            Span::styled("⭐ ".to_string(), Style::default().fg(Color::Yellow)),
            Span::styled(
                "Arrival Curve Table:",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Shows the minimum and maximum lengths of intervals containing N job releases",
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Number of jobs", Style::default().fg(Color::Yellow)),
            Span::raw(" | "),
            Span::styled("Minimum interval", Style::default().fg(Color::Green)),
            Span::raw(" | "),
            Span::styled("Maximum interval", Style::default().fg(Color::Red)),
        ]));

        lines.push(Line::from(format!(
            "  {}",
            "─".repeat(width.saturating_sub(4) as usize)
        )));

        for i in 0..dmins.len().min(dmaxs.len()) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:14}", i + 2), Style::default().fg(Color::Yellow)),
                Span::raw(" | "),
                Span::styled(
                    format!("{:16}", dmins[i]),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(" | "),
                Span::styled(format!("{:16}", dmaxs[i]), Style::default().fg(Color::Red)),
            ]));
        }
    }

    fn apply_horizontal_scroll(
        spans: Vec<Span<'_>>,
        area_width: u16,
        horizontal_offset: usize,
    ) -> Vec<Span<'_>> {
        let available_width = area_width.saturating_sub(4) as usize;

        if available_width == 0 {
            return vec![Span::raw("")];
        }

        let full_text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        let chars: Vec<char> = full_text.chars().collect();

        if horizontal_offset >= chars.len() {
            return vec![Span::raw(" ".repeat(available_width))];
        }

        let mut result_spans = Vec::new();
        let mut current_pos = 0;
        let mut visible_chars_count = 0;

        for span in spans {
            let span_content = span.content.as_ref();
            let span_chars: Vec<char> = span_content.chars().collect();
            let span_len = span_chars.len();

            if current_pos + span_len <= horizontal_offset {
                current_pos += span_len;
                continue;
            }

            let start_in_span = horizontal_offset.saturating_sub(current_pos);

            let remaining_in_area = available_width - visible_chars_count;
            if remaining_in_area == 0 {
                break;
            }

            let chars_to_take = (span_len - start_in_span).min(remaining_in_area);

            if chars_to_take > 0 {
                let visible_content: String = span_chars
                    .iter()
                    .skip(start_in_span)
                    .take(chars_to_take)
                    .collect();

                if !visible_content.is_empty() {
                    result_spans.push(Span::styled(visible_content.clone(), span.style));
                    visible_chars_count += visible_content.chars().count();
                }
            }

            current_pos += span_len;
        }

        if visible_chars_count < available_width {
            result_spans.push(Span::raw(" ".repeat(available_width - visible_chars_count)));
        }

        if result_spans.is_empty() {
            vec![Span::raw(" ".repeat(available_width))]
        } else {
            result_spans
        }
    }
}

#[derive(Default)]
struct ModelInfo {
    period: Option<u64>,
    jitter: Option<u64>,
    raw_data: Value,
    count: Option<u64>,
}
