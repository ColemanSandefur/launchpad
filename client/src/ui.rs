use std::{io, time::Duration};

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::commands::Api;

struct TuiApp {
    services: Vec<(String, String)>, // Name and Status
    list_state: ListState,           // Tracks selected service index
    selected_logs: Vec<String>,      // Logs for the currently selected service
    should_quit: bool,
}

impl TuiApp {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            services: vec![],
            list_state,
            selected_logs: vec![],
            should_quit: false,
        }
    }

    fn next(&mut self) {
        if self.services.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.services.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.services.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.services.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn fix_selection(&mut self) {
        if let Some(i) = self.list_state.selected()
            && i >= self.services.len()
            && !self.services.is_empty()
        {
            self.list_state.select(Some(self.services.len() - 1));
        }
    }

    fn selected_service_name(&self) -> Option<&str> {
        self.list_state
            .selected()
            .and_then(|i| self.services.get(i))
            .map(|(name, _)| name.as_str())
    }

    async fn fetch_status(&mut self, api: &mut Api) {
        let Ok(result) = api.get_status().await else {
            return;
        };

        self.services = result
            .services
            .into_iter()
            .map(|(name, service)| (name, format!("{:?}", service.state)))
            .collect();

        self.services.sort_by(|a, b| a.0.cmp(&b.0));

        self.fix_selection();
    }

    async fn fetch_logs(&mut self, api: &mut Api) {
        let name = match self.selected_service_name() {
            Some(name) => name,
            None => return,
        };

        self.selected_logs = match api.logs(name, None).await {
            Ok(logs) => logs.lines,
            Err(_) => vec!["Failed to fetch logs.".into()],
        };
    }
}

pub async fn run(api: &mut Api) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new();

    // Initial fetch of statuses
    app.fetch_status(api).await;
    app.fetch_logs(api).await;

    let result = run_tui_loop(&mut terminal, &mut app, api).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
    api: &mut Api,
) -> anyhow::Result<()> {
    let mut reader = EventStream::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        terminal.draw(|f| {
            // Split screen vertically: Top half for services list, Bottom half for logs
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Percentage(50),
                    Constraint::Length(3), // Footer
                ])
                .split(f.area());

            // 1. Services List Pane
            let items: Vec<ListItem> = app
                .services
                .iter()
                .map(|(name, status)| ListItem::new(format!("{:<20} : {}", name, status)))
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Managed Services"),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");
            f.render_stateful_widget(list, chunks[0], &mut app.list_state);

            // 2. Logs Pane
            let log_title = format!(" Logs [{}] ", app.selected_service_name().unwrap_or("None"));
            let log_text = app.selected_logs.join("\n");

            // Calculate inner height of the pane (subtract 2 for top/bottom borders)
            let log_area_height = chunks[1].height.saturating_sub(2) as usize;
            let total_lines = app.selected_logs.len();

            // Scroll down so the bottom-most lines are always visible
            let scroll_y = if total_lines > log_area_height {
                (total_lines - log_area_height) as u16
            } else {
                0
            };
            let logs_paragraph = Paragraph::new(log_text)
                .scroll((scroll_y, 0))
                .block(Block::default().borders(Borders::ALL).title(log_title));
            f.render_widget(logs_paragraph, chunks[1]);

            // 3. Footer instructions
            let footer = Paragraph::new(" [↑/↓ or j/k] Navigate | [r] Refresh | [q] Quit")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(footer, chunks[2]);
        })?;

        tokio::select! {
            // Handle terminal input events
            Some(Ok(event)) = reader.next() => {
                if let Event::Key(key) = event {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                        break;
                    }
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.next();
                            app.fetch_logs(api).await;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.previous();
                            app.fetch_logs(api).await;
                        }
                        KeyCode::Char('r') => {
                            app.fetch_status(api).await;
                            app.fetch_logs(api).await;
                        }
                        _ => {}
                    }
                }
            }

            // Periodic background ticker to auto-refresh status & logs
            _ = interval.tick() => {
                app.fetch_status(api).await;
                app.fetch_logs(api).await;
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
