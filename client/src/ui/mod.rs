pub mod home_state;

use std::{io, time::Duration};

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

use crate::{api::Api, ui::home_state::HomeState};

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
            state: TuiState::Home(HomeState::new(api).await),
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

/// Determines the current main "screen" of the application. To add a new option, extend the TuiState
/// with a new struct similar to the format of HomeState
#[derive(Clone, Debug, PartialEq, Eq)]
enum TuiState {
    Home(HomeState),
}
