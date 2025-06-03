use anyhow::Result;
use crossterm::event::KeyCode;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::task::TaskId;

use super::{ProcessList, StatsView, ViewData};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusPanel {
    ProcessList,
    ModelView,
    DetailView,
}

// Main application state containing all UI data and current focus
pub struct AppState {
    pub folder: String,
    pub data: Option<ViewData>,
    pub process_list: Option<ProcessList>,
    pub stats_view: StatsView,
    pub focused_panel: FocusPanel,
}

impl AppState {
    // Creates a new application state for the given folder
    pub fn new(folder: String) -> Self {
        Self {
            folder,
            data: None,
            process_list: None,
            stats_view: StatsView::new(),
            focused_panel: FocusPanel::ProcessList,
        }
    }

    // Loads trace data from the specified folder into the app state
    pub fn load_data(&mut self) -> Result<()> {
        let mut data = ViewData::new();
        data.load_from_folder(&self.folder)?;

        self.process_list = Some(ProcessList::new(data.clone()));
        self.data = Some(data);

        Ok(())
    }

    // Navigate to the next process in the process list
    pub fn next_process(&mut self) {
        if let Some(process_list) = &mut self.process_list {
            process_list.next();
        }
    }

    // Navigate to the previous process in the process list
    pub fn previous_process(&mut self) {
        if let Some(process_list) = &mut self.process_list {
            process_list.previous();
        }
    }

    // Scroll down in the detail view
    pub fn scroll_detail_view_down(&mut self) {
        self.stats_view.scroll_detail_view_down();
    }

    // Scroll up in the detail view
    pub fn scroll_detail_view_up(&mut self) {
        self.stats_view.scroll_detail_view_up();
    }

    pub fn scroll_process_list_left(&mut self, area_width: u16) {
        if let Some(process_list) = &mut self.process_list {
            process_list.scroll_left(area_width);
        }
    }

    pub fn scroll_process_list_right(&mut self, area_width: u16) {
        if let Some(process_list) = &mut self.process_list {
            process_list.scroll_right(area_width);
        }
    }

    pub fn scroll_stats_view_left(&mut self, area_width: u16) {
        self.stats_view.scroll_left(area_width);
    }

    pub fn scroll_stats_view_right(&mut self, area_width: u16) {
        self.stats_view.scroll_right(area_width);
    }

    pub fn process_list_handle_enter(&mut self) {
        if let Some(process_list) = &mut self.process_list {
            if process_list.handle_enter().is_some() {
                self.focused_panel = FocusPanel::ModelView;
            }
        }
    }

    pub fn process_list_handle_escape(&mut self) {
        if let Some(process_list) = &mut self.process_list {
            process_list.handle_escape();
        }
    }

    pub fn stats_view_handle_escape(&mut self) {
        self.stats_view.handle_escape();
    }

    // Get the currently selected process ID
    pub fn get_selected_process(&self) -> Option<TaskId> {
        self.process_list.as_ref().and_then(|pl| pl.get_selected())
    }

    // Navigate to the next separator in the stats view
    pub fn next_separator(&mut self) {
        self.stats_view.next_separator();
    }

    // Navigate to the previous separator in the stats view
    pub fn previous_separator(&mut self) {
        self.stats_view.previous_separator();
    }
}

// Main UI drawing function that renders the entire interface
pub fn draw_ui<B: Backend>(f: &mut Frame<B>, app: &mut AppState) {
    let size = f.size();

    if app.focused_panel == FocusPanel::DetailView {
        if let Some(task_id) = app.get_selected_process() {
            if let Some(process_list) = &app.process_list {
                if let Some(process) = process_list.get_process_info(&task_id) {
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .margin(1)
                        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
                        .split(size);

                    app.stats_view.draw_detail_fullscreen(f, chunks[0], process);

                    draw_help_text_detail_mode(f, chunks[1]);
                    return;
                }
            }
        }

        app.focused_panel = FocusPanel::ModelView;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(size);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .split(chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .split(main_chunks[1]);

    if let Some(process_list) = &mut app.process_list {
        let process_block = Block::default()
            .title("Tasks")
            .borders(Borders::ALL)
            .border_style(if app.focused_panel == FocusPanel::ProcessList {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        process_list.draw(f, main_chunks[0], Some(process_block));
    }

    if let Some(task_id) = app.get_selected_process() {
        if let Some(process_list) = &app.process_list {
            if let Some(process) = process_list.get_process_info(&task_id) {
                let separator_block = Block::default()
                    .title("Separators")
                    .borders(Borders::ALL)
                    .border_style(if app.focused_panel == FocusPanel::ModelView {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    });

                let details_block = Block::default()
                    .title("Separator Details")
                    .borders(Borders::ALL);

                app.stats_view.draw_split(
                    f,
                    right_chunks[0],
                    right_chunks[1],
                    process,
                    Some(separator_block),
                    Some(details_block),
                );
            } else {
                let no_selection = Paragraph::new("Process info not found").block(
                    Block::default()
                        .title("Model Details")
                        .borders(Borders::ALL),
                );
                f.render_widget(no_selection, main_chunks[1]);
            }
        } else {
            let no_selection = Paragraph::new("No process list available").block(
                Block::default()
                    .title("Model Details")
                    .borders(Borders::ALL),
            );
            f.render_widget(no_selection, main_chunks[1]);
        }
    } else {
        let no_selection = Paragraph::new("Select a process to view details").block(
            Block::default()
                .title("Model Details")
                .borders(Borders::ALL),
        );
        f.render_widget(no_selection, main_chunks[1]);
    }

    draw_help_text_normal_mode(f, chunks[1]);
}

// Renders help text for normal mode navigation
fn draw_help_text_normal_mode<B: Backend>(f: &mut Frame<B>, area: Rect) {
    let help_text = vec![
        Line::from(vec![
            Span::styled(
                "Navigation:  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
            Span::raw(" - Scroll horizontally  |  "),
            Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
            Span::raw(" - Select item  |  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" - Go deeper  |  "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" - Go back  |  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" - Quit"),
        ]),
        Line::from(vec![
            Span::styled(
                "Panel Flow: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Tasks → Separators → Details. "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" advances, "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" goes back. "),
            Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
            Span::raw(" scrolls current panel horizontally."),
        ]),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .style(Style::default().fg(Color::White));

    f.render_widget(help_widget, area);
}

// Renders help text for detail view mode
fn draw_help_text_detail_mode<B: Backend>(f: &mut Frame<B>, area: Rect) {
    let help_text = vec![
        Line::from(vec![
            Span::styled(
                "Detail View Mode  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Press "),
            Span::styled("ESC", Style::default().fg(Color::Yellow)),
            Span::raw(" to return to the main view."),
        ]),
        Line::from(vec![
            Span::styled(
                "Note: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("This view provides detailed information about the selected separator."),
        ]),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .style(Style::default().fg(Color::White));

    f.render_widget(help_widget, area);
}

// Handles keyboard input and updates application state accordingly
pub fn handle_input(app: &mut AppState, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') => return false,
        KeyCode::Esc => match app.focused_panel {
            FocusPanel::DetailView => {
                app.focused_panel = FocusPanel::ModelView;
                app.stats_view.reset_detail_scroll();
            }
            FocusPanel::ModelView => {
                app.focused_panel = FocusPanel::ProcessList;
                app.stats_view_handle_escape();
            }
            FocusPanel::ProcessList => {
                app.process_list_handle_escape();
            }
        },
        KeyCode::Enter => match app.focused_panel {
            FocusPanel::ProcessList => {
                app.process_list_handle_enter();
            }
            FocusPanel::ModelView => {
                if app.stats_view.has_separator_selected() {
                    app.focused_panel = FocusPanel::DetailView;
                    app.stats_view.reset_detail_scroll();
                }
            }
            FocusPanel::DetailView => {}
        },
        KeyCode::Down | KeyCode::Char('j') => match app.focused_panel {
            FocusPanel::ProcessList => {
                app.next_process();
            }
            FocusPanel::ModelView => {
                app.stats_view.next_separator();
            }
            FocusPanel::DetailView => {
                app.stats_view.scroll_detail_view_down();
            }
        },
        KeyCode::Up | KeyCode::Char('k') => match app.focused_panel {
            FocusPanel::ProcessList => {
                app.previous_process();
            }
            FocusPanel::ModelView => {
                app.stats_view.previous_separator();
            }
            FocusPanel::DetailView => {
                app.stats_view.scroll_detail_view_up();
            }
        },
        KeyCode::Left | KeyCode::Char('h') => match app.focused_panel {
            FocusPanel::ProcessList => {
                app.scroll_process_list_left(50);
            }
            FocusPanel::ModelView => {
                app.scroll_stats_view_left(50);
            }
            FocusPanel::DetailView => {
                app.scroll_stats_view_left(50);
            }
        },
        KeyCode::Right | KeyCode::Char('l') => match app.focused_panel {
            FocusPanel::ProcessList => {
                app.scroll_process_list_right(50);
            }
            FocusPanel::ModelView => {
                app.scroll_stats_view_right(50);
            }
            FocusPanel::DetailView => {
                app.scroll_stats_view_right(50);
            }
        },
        _ => {}
    }

    true
}
