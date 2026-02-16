use crate::channels::local::session::{
    enqueue_chat_message, is_chat_exit_command, process_message, LocalChatSession,
};
use crate::queue::OutgoingMessage;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const PROCESSING_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
const UI_POLL_INTERVAL: Duration = Duration::from_millis(60);
const SPINNER_TICK_INTERVAL: Duration = Duration::from_millis(120);
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
struct ChatLine {
    speaker: &'static str,
    text: String,
}

struct ProcessingWorker {
    message_id: String,
    result_rx: Receiver<Result<Vec<OutgoingMessage>, String>>,
}

struct TuiState {
    input: String,
    transcript: Vec<ChatLine>,
    processing: Option<ProcessingWorker>,
    spinner_index: usize,
    last_spinner_tick: Instant,
    cursor_visible: bool,
    last_cursor_tick: Instant,
}

impl TuiState {
    fn new(session: &LocalChatSession) -> Self {
        Self {
            input: String::new(),
            transcript: vec![ChatLine {
                speaker: "system",
                text: format!(
                    "chat profile={} conversation_id={}",
                    session.profile_id, session.conversation_id
                ),
            }],
            processing: None,
            spinner_index: 0,
            last_spinner_tick: Instant::now(),
            cursor_visible: true,
            last_cursor_tick: Instant::now(),
        }
    }

    fn spinner_frame(&self) -> &'static str {
        PROCESSING_FRAMES[self.spinner_index % PROCESSING_FRAMES.len()]
    }

    fn advance_spinner_if_needed(&mut self) {
        if self.processing.is_some() && self.last_spinner_tick.elapsed() >= SPINNER_TICK_INTERVAL {
            self.spinner_index = (self.spinner_index + 1) % PROCESSING_FRAMES.len();
            self.last_spinner_tick = Instant::now();
        }
    }

    fn status_line(&self) -> String {
        if let Some(worker) = &self.processing {
            return format!(
                "assistant> thinking {} (message_id={})",
                self.spinner_frame(),
                worker.message_id
            );
        }
        "enter text and press Enter; use /exit to quit".to_string()
    }

    fn advance_cursor_blink_if_needed(&mut self) {
        if self.last_cursor_tick.elapsed() >= CURSOR_BLINK_INTERVAL {
            self.cursor_visible = !self.cursor_visible;
            self.last_cursor_tick = Instant::now();
        }
    }

    fn cursor_suffix(&self) -> &'static str {
        if self.cursor_visible {
            "█"
        } else {
            " "
        }
    }
}

pub fn run_local_chat_session_tui(session: LocalChatSession) -> Result<(), String> {
    let mut terminal = setup_terminal()?;
    let mut state = TuiState::new(&session);

    let result = run_event_loop(&mut terminal, &session, &mut state);
    teardown_terminal(&mut terminal)?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session: &LocalChatSession,
    state: &mut TuiState,
) -> Result<(), String> {
    loop {
        state.advance_spinner_if_needed();
        state.advance_cursor_blink_if_needed();
        check_processing_result(state)?;
        draw_chat_ui(terminal, session, state)?;

        if !event::poll(UI_POLL_INTERVAL).map_err(|e| format!("failed to poll events: {e}"))? {
            continue;
        }

        let Event::Key(key) = event::read().map_err(|e| format!("failed to read event: {e}"))?
        else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
        }

        match key.code {
            KeyCode::Esc => break,
            KeyCode::Enter => {
                let message = state.input.trim().to_string();
                state.input.clear();
                if message.is_empty() {
                    continue;
                }
                if is_chat_exit_command(&message) {
                    break;
                }
                if state.processing.is_some() {
                    state.transcript.push(ChatLine {
                        speaker: "system",
                        text: "still processing previous request".to_string(),
                    });
                    continue;
                }

                state.transcript.push(ChatLine {
                    speaker: "you",
                    text: message.clone(),
                });

                let message_id = enqueue_chat_message(session, &message)?;
                let worker_session = session.clone();
                let worker_message_id = message_id.clone();
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let result = process_message(&worker_session, &worker_message_id);
                    let _ = tx.send(result);
                });

                state.processing = Some(ProcessingWorker {
                    message_id,
                    result_rx: rx,
                });
                state.spinner_index = 0;
                state.last_spinner_tick = Instant::now();
                state.cursor_visible = true;
                state.last_cursor_tick = Instant::now();
            }
            KeyCode::Backspace => {
                state.input.pop();
            }
            KeyCode::Char(c) => {
                state.input.push(c);
            }
            _ => {}
        }
    }

    Ok(())
}

fn check_processing_result(state: &mut TuiState) -> Result<(), String> {
    let Some(worker) = state.processing.take() else {
        return Ok(());
    };

    match worker.result_rx.try_recv() {
        Ok(result) => {
            let responses = result?;
            if responses.is_empty() {
                state.transcript.push(ChatLine {
                    speaker: "assistant",
                    text: format!(
                        "timed out waiting for response (message_id={})",
                        worker.message_id
                    ),
                });
            } else {
                for response in responses {
                    state.transcript.push(ChatLine {
                        speaker: "assistant",
                        text: response.message,
                    });
                }
            }
        }
        Err(mpsc::TryRecvError::Empty) => {
            state.processing = Some(worker);
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            return Err("chat processing worker disconnected unexpectedly".to_string());
        }
    }

    Ok(())
}

fn draw_chat_ui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    session: &LocalChatSession,
    state: &TuiState,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .split(frame.area());

            let header = Paragraph::new(vec![
                Line::raw("DireClaw Local Chat"),
                Line::raw(format!(
                    "profile={} conversation_id={}",
                    session.profile_id, session.conversation_id
                )),
            ])
            .block(
                Block::default()
                    .title("Session")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            frame.render_widget(header, sections[0]);

            let transcript = state
                .transcript
                .iter()
                .map(|line| {
                    if line.speaker == "assistant" {
                        Line::styled(
                            format!("{}> {}", line.speaker, line.text),
                            Style::default().fg(Color::Green),
                        )
                    } else if line.speaker == "you" {
                        Line::styled(
                            format!("{}> {}", line.speaker, line.text),
                            Style::default().fg(Color::Yellow),
                        )
                    } else {
                        Line::styled(
                            format!("{}> {}", line.speaker, line.text),
                            Style::default().fg(Color::Gray),
                        )
                    }
                })
                .collect::<Vec<_>>();
            let transcript_widget = Paragraph::new(transcript)
                .block(Block::default().title("Transcript").borders(Borders::ALL))
                .wrap(Wrap { trim: false });
            frame.render_widget(transcript_widget, sections[1]);

            let status_widget = Paragraph::new(state.status_line()).block(
                Block::default()
                    .title("Status")
                    .borders(Borders::ALL)
                    .border_style(if state.processing.is_some() {
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    }),
            );
            frame.render_widget(status_widget, sections[2]);

            let input_widget =
                Paragraph::new(format!("you> {}{}", state.input, state.cursor_suffix()))
                    .block(Block::default().title("Input").borders(Borders::ALL));
            frame.render_widget(input_widget, sections[3]);
        })
        .map_err(|e| format!("failed to render local chat UI: {e}"))?;

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, String> {
    enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {e}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)
        .map_err(|e| format!("failed to enter alternate screen: {e}"))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|e| format!("failed to initialize terminal: {e}"))
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
    disable_raw_mode().map_err(|e| format!("failed to disable raw mode: {e}"))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)
        .map_err(|e| format!("failed to leave alternate screen: {e}"))?;
    terminal
        .show_cursor()
        .map_err(|e| format!("failed to restore cursor: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TuiState, CURSOR_BLINK_INTERVAL, PROCESSING_FRAMES};
    use crate::channels::local::session::LocalChatSession;
    use crate::config::{ChannelKind, ChannelProfile, Settings};
    use crate::queue::QueuePaths;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Instant;

    fn fake_session() -> LocalChatSession {
        LocalChatSession {
            state_root: PathBuf::from("/tmp/state"),
            settings: Settings {
                workspaces_path: PathBuf::from("/tmp/workspaces"),
                shared_workspaces: BTreeMap::new(),
                orchestrators: BTreeMap::new(),
                channel_profiles: BTreeMap::new(),
                monitoring: Default::default(),
                channels: BTreeMap::new(),
                auth_sync: Default::default(),
            },
            queue_paths: QueuePaths::from_state_root(PathBuf::from("/tmp/state").as_path()),
            profile_id: "local-default".to_string(),
            profile: ChannelProfile {
                channel: ChannelKind::Local,
                orchestrator_id: "eng".to_string(),
                slack_app_user_id: None,
                require_mention_in_channels: None,
            },
            conversation_id: "chat-1".to_string(),
        }
    }

    #[test]
    fn spinner_frame_cycles_across_ascii_frames() {
        let session = fake_session();
        let mut state = TuiState::new(&session);
        assert_eq!(state.spinner_frame(), PROCESSING_FRAMES[0]);
        state.spinner_index = 1;
        assert_eq!(state.spinner_frame(), PROCESSING_FRAMES[1]);
        state.spinner_index = 2;
        assert_eq!(state.spinner_frame(), PROCESSING_FRAMES[2]);
        state.spinner_index = 3;
        assert_eq!(state.spinner_frame(), PROCESSING_FRAMES[3]);
    }

    #[test]
    fn cursor_blink_toggles_visibility_after_interval() {
        let session = fake_session();
        let mut state = TuiState::new(&session);
        assert_eq!(state.cursor_suffix(), "█");

        state.last_cursor_tick = Instant::now() - CURSOR_BLINK_INTERVAL;
        state.advance_cursor_blink_if_needed();
        assert_eq!(state.cursor_suffix(), " ");
    }
}
