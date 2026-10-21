use std::{io, time::Duration};

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::commands::Api;

pub async fn run(api: &mut Api) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new(api).await;

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
        terminal.draw(|f| app.render(f))?;

        tokio::select! {
            // Handle terminal input events
            Some(Ok(event)) = reader.next() => {
                if let Event::Key(key) = event {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                        break;
                    }
                    match key.code {
                        KeyCode::Char('q') => break,
                        _ => {}
                    };
                }

                app.key_event(api, &event).await;
            }

            // Periodic background ticker to auto-refresh status & logs
            _ = interval.tick() => {
                app.update(api).await;
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

struct TuiApp {
    state: TuiState,
    should_quit: bool,
}

impl TuiApp {
    async fn new(api: &mut Api) -> Self {
        Self {
            state: TuiState::Home(TuiListState::new(api).await),
            should_quit: false,
        }
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        match &mut self.state {
            TuiState::Home(api_list_state) => {
                api_list_state.render(f);
            }
        }
    }

    async fn key_event(&mut self, api: &mut Api, event: &Event) {
        match &mut self.state {
            TuiState::Home(api_list_state) => {
                api_list_state.key_event(api, &event).await;
            }
        }
    }

    async fn update(&mut self, api: &mut Api) {
        match &mut self.state {
            TuiState::Home(api_list_state) => {
                api_list_state.update(api).await;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TuiState {
    Home(TuiListState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TuiListState {
    services: Vec<(String, String)>, // Name and Status
    list_state: ListState,           // Tracks selected service index
    selected_logs: Vec<String>,      // Logs for the currently selected service
}

impl TuiListState {
    pub async fn new(api: &mut Api) -> Self {
        let mut state = Self {
            services: vec![],
            list_state: Default::default(),
            selected_logs: Default::default(),
        };

        state.update(api).await;

        state
    }

    fn render(&mut self, f: &mut Frame<'_>) {
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
        let items: Vec<ListItem> = self
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
        f.render_stateful_widget(list, chunks[0], &mut self.list_state);

        // 2. Logs Pane
        let log_title = format!(
            " Logs [{}] ",
            self.selected_service_name().unwrap_or("None")
        );
        let log_text = self.selected_logs.join("\n");

        // Calculate inner height of the pane (subtract 2 for top/bottom borders)
        let log_area_height = chunks[1].height.saturating_sub(2) as usize;
        let total_lines = self.selected_logs.len();

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
    }

    async fn key_event(&mut self, api: &mut Api, event: &Event) {
        // Handle terminal input events
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.next();
                    self.fetch_logs(api).await;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.previous();
                    self.fetch_logs(api).await;
                }
                KeyCode::Char('r') => {
                    self.fetch_status(api).await;
                    self.fetch_logs(api).await;
                }
                _ => {}
            }
        }
    }
    async fn update(&mut self, api: &mut Api) {
        self.fetch_status(api).await;
        self.fetch_logs(api).await;
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
