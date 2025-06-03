use tui::{
    backend::Backend,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

use crate::events::SchedulingPolicy;
use crate::task::TaskId;

use super::data::{ProcessInfo, ViewData};

// Manages the process list view with navigation and rendering
pub struct ProcessList {
    data: ViewData,
    process_ids: Vec<TaskId>,
    state: TableState,
    horizontal_offset: usize,
}

impl ProcessList {
    // Creates a new process list from view data
    pub fn new(data: ViewData) -> Self {
        let process_ids: Vec<TaskId> = data.get_processes().keys().cloned().collect();
        let mut state = TableState::default();
        if !process_ids.is_empty() {
            state.select(Some(0));
        }

        Self {
            data,
            process_ids,
            state,
            horizontal_offset: 0,
        }
    }

    // Returns true if there are no processes to display
    pub fn is_empty(&self) -> bool {
        self.process_ids.is_empty()
    }

    // Navigate to the next process in the list
    pub fn next(&mut self) {
        if self.process_ids.is_empty() {
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.process_ids.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    // Navigate to the previous process in the list
    pub fn previous(&mut self) {
        if self.process_ids.is_empty() {
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.process_ids.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn scroll_left(&mut self, _area_width: u16) {
        self.horizontal_offset = self.horizontal_offset.saturating_sub(10);
    }

    pub fn scroll_right(&mut self, _area_width: u16) {
        self.horizontal_offset += 10;
    }

    pub fn handle_enter(&self) -> Option<TaskId> {
        self.get_selected()
    }

    pub fn handle_escape(&mut self) {
        self.horizontal_offset = 0;
    }

    // Get the currently selected process ID
    pub fn get_selected(&self) -> Option<TaskId> {
        self.state
            .selected()
            .and_then(|i| self.process_ids.get(i).cloned())
    }

    // Get process information for a specific task ID
    pub fn get_process_info(&self, task_id: &TaskId) -> Option<&ProcessInfo> {
        self.data.get_process(task_id)
    }

    // Renders the process list table to the terminal
    pub fn draw<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        area: Rect,
        custom_block: Option<Block<'_>>,
    ) {
        let processes = self.data.get_processes();

        let column_widths = [8, 12, 6, 16, 40];
        let column_titles = ["Task ID", "Policy", "Prio", "Comm", "Command"];

        let mut cumulative_width = 0;
        let mut visible_columns = Vec::new();
        let mut visible_constraints = Vec::new();

        for (i, &width) in column_widths.iter().enumerate() {
            let col_start = cumulative_width;
            let col_end = cumulative_width + width;

            if col_end > self.horizontal_offset
                && col_start < self.horizontal_offset + area.width as usize
            {
                let visible_start = if col_start < self.horizontal_offset {
                    self.horizontal_offset.saturating_sub(col_start)
                } else {
                    0
                };

                let visible_end = if col_end > self.horizontal_offset + area.width as usize {
                    (self.horizontal_offset + area.width as usize) - col_start
                } else {
                    width
                };

                let visible_width = visible_end.saturating_sub(visible_start);

                if visible_width > 0 {
                    visible_columns.push((i, visible_start, visible_width));
                    visible_constraints.push(Constraint::Length(visible_width as u16));
                }
            }

            cumulative_width += width + 1;
        }

        if visible_columns.is_empty() {
            visible_columns.push((0, 0, 1));
            visible_constraints.push(Constraint::Length(1));
        }

        let header_cells: Vec<Cell> = visible_columns
            .iter()
            .map(|(col_idx, start, width)| {
                let title = column_titles[*col_idx];
                let truncated = if *start < title.len() {
                    let end = (*start + *width).min(title.len());
                    &title[*start..end]
                } else {
                    ""
                };
                Cell::from(format!("{:width$}", truncated, width = *width))
                    .style(Style::default().fg(Color::Yellow))
            })
            .collect();

        let header = Row::new(header_cells).style(Style::default()).height(1);

        let rows = self.process_ids.iter().map(|id| {
            let process = processes.get(id).unwrap();

            let task_id_str = format!("{}", id);
            let policy_str = match process.info.policy {
                SchedulingPolicy::Other => "CFS".to_string(),
                SchedulingPolicy::Fifo { .. } => "FIFO".to_string(),
                SchedulingPolicy::RoundRobin { .. } => "RR".to_string(),
                SchedulingPolicy::Deadline {
                    runtime,
                    period,
                    deadline,
                } => format!("DL {}/{}/{}", runtime, period, deadline),
                SchedulingPolicy::Unknown => "?".to_string(),
            };

            let prio_str = match process.info.policy {
                SchedulingPolicy::Fifo { prio } => format!("{}", prio),
                SchedulingPolicy::RoundRobin { prio } => format!("{}", prio),
                _ => "-".to_string(),
            };

            let comm = process
                .info
                .comm
                .clone()
                .unwrap_or_else(|| String::from("-"));

            let cmd = process
                .info
                .cmd
                .clone()
                .unwrap_or_else(|| String::from("-"));

            let column_data = [&task_id_str, &policy_str, &prio_str, &comm, &cmd];

            let cells: Vec<Cell> = visible_columns
                .iter()
                .map(|(col_idx, start, width)| {
                    let text = column_data[*col_idx];
                    let truncated = if *start < text.len() {
                        let end = (*start + *width).min(text.len());
                        &text[*start..end]
                    } else {
                        ""
                    };
                    Cell::from(format!("{:width$}", truncated, width = *width))
                })
                .collect();

            Row::new(cells)
        });

        let block =
            custom_block.unwrap_or_else(|| Block::default().title("Tasks").borders(Borders::ALL));

        let table = Table::new(rows)
            .header(header)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ")
            .widths(&visible_constraints);

        f.render_stateful_widget(table, area, &mut self.state);
    }
}
