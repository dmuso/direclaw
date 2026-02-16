use crate::channels::local::session::{
    enqueue_chat_message, is_chat_exit_command, process_message, LocalChatSession,
};
use crate::orchestration::progress::ProgressSnapshot;
use crate::orchestration::run_store::RunState;
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
use std::collections::HashSet;
use std::fs;
use std::io::{self, Stdout};
use std::path::Path;
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
    run_id: Option<String>,
    last_progress_line: Option<String>,
    seen_assistant_lines: HashSet<String>,
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
        check_processing_result(session, state)?;
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
                    run_id: None,
                    last_progress_line: None,
                    seen_assistant_lines: HashSet::new(),
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

fn check_processing_result(session: &LocalChatSession, state: &mut TuiState) -> Result<(), String> {
    let Some(worker) = state.processing.take() else {
        return Ok(());
    };
    let mut worker = worker;

    maybe_emit_progress_update(&session.state_root, state, &mut worker)?;

    match worker.result_rx.try_recv() {
        Ok(result) => {
            let responses = result?;
            if responses.is_empty() {
                let message_id = worker.message_id.clone();
                push_assistant_line(
                    state,
                    &mut worker,
                    format!("timed out waiting for response (message_id={})", message_id),
                );
            } else {
                for response in responses {
                    push_assistant_line(state, &mut worker, response.message);
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

fn push_assistant_line(state: &mut TuiState, worker: &mut ProcessingWorker, text: String) {
    if !worker.seen_assistant_lines.insert(text.clone()) {
        return;
    }
    state.transcript.push(ChatLine {
        speaker: "assistant",
        text,
    });
}

fn maybe_emit_progress_update(
    state_root: &Path,
    state: &mut TuiState,
    worker: &mut ProcessingWorker,
) -> Result<(), String> {
    if worker.run_id.is_none() {
        worker.run_id = find_run_id_for_message(state_root, &worker.message_id)?;
    }
    let Some(run_id) = worker.run_id.as_deref() else {
        return Ok(());
    };
    let progress_path = state_root
        .join("workflows/runs")
        .join(run_id)
        .join("progress.json");
    if !progress_path.is_file() {
        return Ok(());
    }
    let raw = fs::read_to_string(&progress_path)
        .map_err(|e| format!("failed to read {}: {e}", progress_path.display()))?;
    let progress: ProgressSnapshot = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse {}: {e}", progress_path.display()))?;
    let Some(line) = render_progress_line(&progress) else {
        return Ok(());
    };
    if worker.last_progress_line.as_deref() == Some(line.as_str()) {
        return Ok(());
    }
    worker.last_progress_line = Some(line.clone());
    push_assistant_line(state, worker, line);
    Ok(())
}

fn find_run_id_for_message(state_root: &Path, message_id: &str) -> Result<Option<String>, String> {
    let runs_root = state_root.join("workflows/runs");
    if !runs_root.is_dir() {
        return Ok(None);
    }
    let entries = fs::read_dir(&runs_root)
        .map_err(|e| format!("failed to read {}: {e}", runs_root.display()))?;
    let mut latest: Option<(i64, String)> = None;

    for entry in entries {
        let entry =
            entry.map_err(|e| format!("failed to read {} entry: {e}", runs_root.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }

        let raw = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let source_message_id = parsed
            .get("sourceMessageId")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("source_message_id").and_then(|v| v.as_str()));
        if source_message_id != Some(message_id) {
            continue;
        }
        let run_id = parsed
            .get("runId")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("run_id").and_then(|v| v.as_str()))
            .unwrap_or_default()
            .to_string();
        if run_id.is_empty() {
            continue;
        }
        let started_at = parsed
            .get("startedAt")
            .and_then(|v| v.as_i64())
            .or_else(|| parsed.get("started_at").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        let should_replace = latest
            .as_ref()
            .map(|(current_started, _)| started_at >= *current_started)
            .unwrap_or(true);
        if should_replace {
            latest = Some((started_at, run_id));
        }
    }

    Ok(latest.map(|(_, run_id)| run_id))
}

fn render_progress_line(progress: &ProgressSnapshot) -> Option<String> {
    match progress.state {
        RunState::Running => {
            let step_id = progress.current_step_id.as_deref()?;
            let attempt = progress.current_attempt.unwrap_or(1);
            Some(format!("Running step `{step_id}` (attempt {attempt})..."))
        }
        RunState::Failed => Some(progress.summary.clone()),
        RunState::Canceled => Some(progress.summary.clone()),
        _ => None,
    }
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
    use super::{render_progress_line, TuiState, CURSOR_BLINK_INTERVAL, PROCESSING_FRAMES};
    use crate::channels::local::session::LocalChatSession;
    use crate::config::{ChannelKind, ChannelProfile, Settings};
    use crate::orchestration::progress::ProgressSnapshot;
    use crate::orchestration::run_store::RunState;
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

    #[test]
    fn running_progress_renders_step_status_line() {
        let progress = ProgressSnapshot {
            run_id: "run-1".to_string(),
            workflow_id: "quick_answer".to_string(),
            state: RunState::Running,
            input_count: 1,
            input_keys: vec!["message".to_string()],
            current_step_id: Some("answer".to_string()),
            current_attempt: Some(1),
            started_at: 100,
            updated_at: 101,
            last_progress_at: 101,
            summary: "step answer attempt 1 running".to_string(),
            pending_human_input: false,
            next_expected_action: "await step output".to_string(),
        };

        let rendered = render_progress_line(&progress).expect("line");
        assert_eq!(rendered, "Running step `answer` (attempt 1)...");
    }
}
