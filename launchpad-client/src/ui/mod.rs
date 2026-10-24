pub mod home_state;
pub mod log_state;

use std::{io, time::Duration};

use bb8::Pool;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use tokio::sync::watch;

use crate::{
    channel_id::ChannelId,
    connection_manager::ConnectionManager,
    ui::{home_state::HomeState, log_state::LogState},
};

pub async fn run(id: ChannelId) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new(id).await?;

    let result = run_tui_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
) -> anyhow::Result<()> {
    let mut reader = EventStream::new();
    let mut interval = tokio::time::interval(Duration::from_millis(16));

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

                app.key_event(&event).await;
            }

            _ = app.events_ready() => {
                app.handle_events();
            }

            // Periodic background ticker to auto-refresh status & logs
            _ = interval.tick() => {
                app.tick();
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

pub enum TuiEvent {
    ViewLog(String),
    ViewHome,
}

struct TuiApp {
    state: TuiState,
    should_quit: bool,
    pool: Pool<ConnectionManager>,
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl TuiApp {
    async fn new(id: ChannelId) -> anyhow::Result<Self> {
        let mgr = ConnectionManager::new(id);
        let pool = bb8::Pool::builder().max_size(10).build(mgr).await?;
        let (tx, rx) = watch::channel(false);
        Ok(Self {
            state: TuiState::Home(HomeState::new(pool.clone(), tx.clone())),
            should_quit: false,
            pool,
            tx,
            rx,
        })
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        match &mut self.state {
            TuiState::Home(api_list_state) => api_list_state.render(f),
            TuiState::Logs(log_state) => log_state.render(f),
        };
    }

    async fn key_event(&mut self, event: &Event) {
        match &mut self.state {
            TuiState::Home(api_list_state) => api_list_state.key_event(event).await,
            TuiState::Logs(log_state) => log_state.key_event(event).await,
        };
    }

    fn has_events(&self) -> bool {
        match &self.state {
            TuiState::Home(home_state) => home_state.has_events(),
            TuiState::Logs(log_state) => log_state.has_events(),
        }
    }

    async fn events_ready(&mut self) {
        let _ = self.rx.changed().await;
    }

    fn handle_events(&mut self) {
        while self.has_events() {
            self.handle_event();
        }
    }

    fn handle_event(&mut self) {
        let event = match &mut self.state {
            TuiState::Home(home_state) => home_state.handle_event(),
            TuiState::Logs(log_state) => log_state.handle_event(),
        };

        match event {
            Some(event) => match event {
                TuiEvent::ViewLog(service_name) => {
                    self.state = TuiState::Logs(LogState::new(
                        self.pool.clone(),
                        self.tx.clone(),
                        service_name,
                    ))
                }
                TuiEvent::ViewHome => {
                    self.state = TuiState::Home(HomeState::new(self.pool.clone(), self.tx.clone()))
                }
            },
            None => (),
        }
    }

    fn tick(&mut self) {
        match &mut self.state {
            TuiState::Home(api_list_state) => api_list_state.tick(),
            TuiState::Logs(log_state) => log_state.tick(),
        };
    }
}

/// Determines the current main "screen" of the application. To add a new option, extend the TuiState
/// with a new struct similar to the format of HomeState
#[derive(Debug)]
enum TuiState {
    Home(HomeState),
    Logs(LogState),
}
