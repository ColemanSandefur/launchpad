use bb8::Pool;
use crossterm::event::{Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph},
};
use tokio::sync::{mpsc, watch};

use crate::{connection_manager::ConnectionManager, ui::TuiEvent};

enum LogMessage {
    LogsUpdate(Vec<String>),
    ViewHome,
}

#[derive(Debug)]
pub struct LogState {
    pool: Pool<ConnectionManager>,
    service_name: String,
    logs: Vec<String>,
    tx: mpsc::Sender<LogMessage>,
    rx: mpsc::Receiver<LogMessage>,
    signal: watch::Sender<bool>,
}

impl LogState {
    pub fn new(
        pool: Pool<ConnectionManager>,
        signal: watch::Sender<bool>,
        service_name: String,
    ) -> Self {
        let (tx, rx) = mpsc::channel(100);

        let mut state = Self {
            pool,
            service_name,
            logs: vec![],
            tx,
            rx,
            signal,
        };

        state.tick();

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

        // 2. Logs Pane
        let log_title = format!(" Logs [{}] ", self.service_name);
        let log_text = self.logs.join("\n");

        // Calculate inner height of the pane (subtract 2 for top/bottom borders)
        let log_area_height = chunks[0].height.saturating_sub(2) as usize;
        let total_lines = self.logs.len();

        // Scroll down so the bottom-most lines are always visible
        let scroll_y = if total_lines > log_area_height {
            (total_lines - log_area_height) as u16
        } else {
            0
        };
        let logs_paragraph = Paragraph::new(log_text)
            .scroll((scroll_y, 0))
            .block(Block::default().borders(Borders::ALL).title(log_title));
        f.render_widget(logs_paragraph, chunks[0]);

        // 3. Footer instructions
        let footer = Paragraph::new(" [↑/↓ or j/k] Navigate | [r] Refresh | [b] Back | [q] Quit")
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(footer, chunks[1]);
    }

    pub async fn key_event(&mut self, event: &Event) {
        // Handle terminal input events
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('b') => {
                    let _ = self.tx.send(LogMessage::ViewHome).await;
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
                LogMessage::LogsUpdate(logs) => {
                    self.logs = logs;
                    None
                }
                LogMessage::ViewHome => Some(TuiEvent::ViewHome),
            },
            Err(_) => None,
        }
    }

    pub fn tick(&mut self) {
        self.fetch_logs();
    }

    fn fetch_logs(&mut self) {
        let tx = self.tx.clone();
        let pool = self.pool.clone();
        let signal = self.signal.clone();

        let name = self.service_name.clone();

        tokio::spawn(async move {
            let Ok(mut api) = pool.get().await else {
                return;
            };

            let selected_logs = match api.logs(name, None).await {
                Ok(logs) => logs.lines,
                Err(_) => vec!["Failed to fetch logs.".into()],
            };

            let _ = tx.send(LogMessage::LogsUpdate(selected_logs)).await;
            let _ = signal.send(true);
        });
    }
}
