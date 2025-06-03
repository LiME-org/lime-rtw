use std::io::stdout;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tui::{
    backend::{Backend, CrosstermBackend},
    widgets::{Block, Borders},
    Terminal,
};

use crate::context::LimeContext;
use crate::events::TraceEvent;
use crate::task::TaskId;
use crate::EventProcessor;

mod data;
mod process_list;
mod stats;
mod ui;

pub use data::ViewData;
pub use process_list::ProcessList;
pub use stats::StatsView;
pub use ui::{draw_ui, handle_input, AppState};

// Main view processor that manages the TUI application lifecycle
pub struct ViewProcessor {
    pub folder: String,
}

impl ViewProcessor {
    // Creates a new view processor for the specified folder
    pub fn new(folder: &str) -> Self {
        Self {
            folder: folder.to_string(),
        }
    }

    // Runs the TUI application with terminal setup and cleanup
    pub fn run(&self, _ctx: &LimeContext) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let app = AppState::new(self.folder.clone());

        let res = self.run_app(&mut terminal, app);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        if let Err(err) = res {
            println!("Error: {:?}", err);
        }

        Ok(())
    }

    // Main application loop that handles drawing and input
    fn run_app<B: Backend>(&self, terminal: &mut Terminal<B>, mut app: AppState) -> Result<()> {
        app.load_data()?;

        if let Some(process_list) = &app.process_list {
            if process_list.is_empty() {
                terminal.draw(|f| {
                    let size = f.size();
                    let block = Block::default().title("No Tasks").borders(Borders::ALL);
                    let text = ["No tasks found to display.", "", "Press Enter to exit..."];
                    let paragraph = tui::widgets::Paragraph::new(text.join("\n"))
                        .block(block)
                        .alignment(tui::layout::Alignment::Center);
                    f.render_widget(paragraph, size);
                })?;

                // Wait for Enter key
                loop {
                    if let Event::Key(key) = event::read()? {
                        if key.code == crossterm::event::KeyCode::Enter {
                            break;
                        }
                    }
                }
                return Ok(());
            }
        }

        loop {
            terminal.draw(|f| draw_ui(f, &mut app))?;

            if let Event::Key(key) = event::read()? {
                if !handle_input(&mut app, key.code) {
                    break;
                }
            }
        }

        Ok(())
    }
}

impl EventProcessor for ViewProcessor {
    fn pre_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        Ok(())
    }

    fn post_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        Ok(())
    }

    fn consume_event(&mut self, _task_id: &TaskId, _event: TraceEvent, _ctx: &LimeContext) {}

    fn finalize<S: crate::EventSource>(&mut self, _src: &S, ctx: &LimeContext) -> Result<()> {
        self.run(ctx)
    }
}
