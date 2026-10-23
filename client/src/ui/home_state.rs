use bb8::Pool;
use common::message::ServiceState;
use crossterm::event::{Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use tokio::sync::{mpsc, watch};

use crate::{connection_manager::ConnectionManager, ui::TuiEvent};

enum HomeMessage {
    Services(Vec<(String, ServiceState)>),
    LogsUpdate(Vec<String>),
    ViewLog(String),
}

#[derive(Debug)]
pub struct HomeState {
    pool: Pool<ConnectionManager>,
    services: Vec<(String, ServiceState)>, // Name and Status
    list_state: TableState,                // Tracks selected service index
    selected_logs: Vec<String>,            // Logs for the currently selected service
    tx: mpsc::Sender<HomeMessage>,
    rx: mpsc::Receiver<HomeMessage>,
    signal: watch::Sender<bool>,
}

impl HomeState {
    pub fn new(pool: Pool<ConnectionManager>, signal: watch::Sender<bool>) -> Self {
        let (tx, rx) = mpsc::channel(100);

        let mut state = Self {
            pool,
            services: vec![],
            list_state: Default::default(),
            selected_logs: Default::default(),
            tx,
            rx,
            signal,
        };

        state.tick();

        state.list_state.select_first();

        state
    }

    pub fn render(&mut self, f: &mut Frame<'_>) {
        // Split screen vertically: Top half for services list, Bottom half for logs
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(100),
                Constraint::Length(3), // Footer
            ])
            .split(f.area());

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Fill(1)])
            .split(chunks[0]);

        self.render_services(f, content[0]);
        self.render_logs(f, content[1]);

        let footer =
            Paragraph::new(" [↑/↓ or j/k] Navigate | [r] Refresh | [v] View Logs | [q] Quit")
                .block(Block::default().borders(Borders::ALL));
        f.render_widget(footer, chunks[1]);
    }

    fn render_services(&mut self, f: &mut Frame<'_>, area: Rect) {
        let rows: Vec<Row> = self
            .services
            .iter()
            .map(|(name, status)| {
                // Optional: Style the status cell based on its value
                let status_color = match status {
                    ServiceState::Healthy => Color::Green,
                    ServiceState::Failed | ServiceState::Unhealthy => Color::Red,
                    ServiceState::Stopped => Color::LightMagenta,
                    _ => Color::Yellow,
                };

                Row::new(vec![
                    Cell::from(name.clone()),
                    Cell::from(format!("{:?}", status)).style(Style::default().fg(status_color)),
                ])
            })
            .collect();

        let table = Table::new(rows, [Constraint::Fill(1), Constraint::Length(8)])
            .header(
                Row::new(vec!["Service Name", "Status"])
                    .style(Style::default().add_modifier(Modifier::BOLD))
                    .bottom_margin(1),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Managed Services"),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(table, area, &mut self.list_state);
    }

    fn render_logs(&self, f: &mut Frame<'_>, area: Rect) {
        let log_title = format!(
            " Logs [{}] ",
            self.selected_service_name().unwrap_or("None")
        );
        let log_text = self.selected_logs.join("\n");

        // Calculate inner height of the pane (subtract 2 for top/bottom borders)
        let log_area_height = area.height.saturating_sub(2) as usize;
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
        f.render_widget(logs_paragraph, area);
    }

    pub async fn key_event(&mut self, event: &Event) {
        // Handle terminal input events
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.next();
                    self.fetch_logs();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.previous();
                    self.fetch_logs();
                }
                KeyCode::Char('r') => {
                    self.fetch_status();
                    self.fetch_logs();
                }
                KeyCode::Char('v') => {
                    if let Some(name) = self.selected_service_name() {
                        let _ = self.tx.send(HomeMessage::ViewLog(name.to_string())).await;
                    }
                }
                _ => {}
            }
        }
    }

    pub fn has_events(&self) -> bool {
        !self.rx.is_empty()
    }

    pub fn handle_event(&mut self) -> Option<TuiEvent> {
        match self.rx.try_recv() {
            Ok(message) => match message {
                HomeMessage::LogsUpdate(logs) => {
                    self.selected_logs = logs;
                    None
                }
                HomeMessage::Services(services) => {
                    self.services = services;
                    self.fix_selection();
                    None
                }
                HomeMessage::ViewLog(log_name) => Some(TuiEvent::ViewLog(log_name)),
            },
            Err(_) => None,
        }
    }

    pub fn tick(&mut self) {
        self.fetch_status();
        self.fetch_logs();
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

        if self.list_state.selected().is_none() {
            self.list_state.select_first();
        }
    }

    fn selected_service_name(&self) -> Option<&str> {
        self.list_state
            .selected()
            .and_then(|i| self.services.get(i))
            .map(|(name, _)| name.as_str())
    }

    fn fetch_status(&self) {
        let tx = self.tx.clone();
        let pool = self.pool.clone();
        let signal = self.signal.clone();

        tokio::spawn(async move {
            let Ok(mut api) = pool.get().await else {
                return;
            };

            let Ok(result) = api.get_status().await else {
                return;
            };

            drop(api);

            let mut services: Vec<_> = result
                .services
                .into_iter()
                .map(|(name, status)| (name, status.state))
                .collect();

            services.sort_by(|a, b| a.0.cmp(&b.0));

            let _ = tx.send(HomeMessage::Services(services)).await;
            let _ = signal.send(true);
        });
    }

    fn fetch_logs(&mut self) {
        let tx = self.tx.clone();
        let pool = self.pool.clone();
        let signal = self.signal.clone();

        let name = match self.selected_service_name() {
            Some(name) => name.to_string(),
            None => return,
        };

        tokio::spawn(async move {
            let Ok(mut api) = pool.get().await else {
                return;
            };

            let selected_logs = match api.logs(name, None).await {
                Ok(logs) => logs.lines,
                Err(_) => vec!["Failed to fetch logs.".into()],
            };

            let _ = tx.send(HomeMessage::LogsUpdate(selected_logs)).await;
            let _ = signal.send(true);
        });
    }
}
