use crate::setup::navigation::{clamp_selection, NavState, ALL_SETUP_SCREENS};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Padding, Paragraph, Row, Table};
use ratatui::{Frame, Terminal};
use std::io;

pub const SETUP_MENU_ITEMS: [&str; 5] = [
    "Workspaces",
    "Orchestrators",
    "Initial Agent Defaults",
    "Save Setup",
    "Cancel",
];

pub struct SetupMenuViewModel {
    pub mode_line: String,
    pub items: Vec<String>,
    pub selected: usize,
    pub status_text: String,
    pub hint_text: String,
}

pub fn project_setup_menu_view_model(config_exists: bool, state: &NavState) -> SetupMenuViewModel {
    debug_assert!(ALL_SETUP_SCREENS.contains(&state.screen));
    SetupMenuViewModel {
        mode_line: if config_exists {
            "Mode: existing setup (edit + apply)".to_string()
        } else {
            "Mode: first-time setup".to_string()
        },
        items: SETUP_MENU_ITEMS
            .iter()
            .map(|item| (*item).to_string())
            .collect(),
        selected: clamp_selection(state.selected, SETUP_MENU_ITEMS.len()),
        status_text: state.status_text.clone(),
        hint_text: state.hint_text.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupFieldRow {
    pub field: String,
    pub value: Option<String>,
}

pub fn field_row(field: &str, value: Option<String>) -> SetupFieldRow {
    SetupFieldRow {
        field: field.to_string(),
        value,
    }
}

pub fn tail_for_display(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub(crate) fn draw_field_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    config_exists: bool,
    rows: &[SetupFieldRow],
    selected: usize,
    status: &str,
    hint: &str,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(4),
                ])
                .split(frame.area());
            let header = Paragraph::new(vec![
                Line::from(Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(if config_exists {
                    "Mode: existing setup"
                } else {
                    "Mode: first-time setup"
                }),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            let table_rows = rows.iter().enumerate().map(|(idx, row)| {
                let style = if idx == selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(row.field.clone()),
                    Cell::from(row.value.clone().unwrap_or_default()),
                ])
                .style(style)
            });
            let table = Table::new(
                table_rows,
                [Constraint::Percentage(45), Constraint::Percentage(55)],
            )
            .column_spacing(2)
            .block(main_panel_block());
            frame.render_widget(table, chunks[1]);

            let footer = Paragraph::new(vec![
                Line::from(hint.to_string()),
                Line::from(format!("Status: {status}")),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(footer, chunks[2]);
        })
        .map_err(|e| format!("failed to render field screen: {e}"))?;
    Ok(())
}

pub(crate) fn draw_list_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    config_exists: bool,
    items: &[String],
    selected: usize,
    status: &str,
    hint: &str,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(4),
                ])
                .split(frame.area());
            let header = Paragraph::new(vec![
                Line::from(Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(if config_exists {
                    "Mode: existing setup"
                } else {
                    "Mode: first-time setup"
                }),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            let mut list_items = Vec::with_capacity(items.len());
            for (idx, line) in items.iter().enumerate() {
                let mut item = ListItem::new(Line::from(Span::raw(line.clone())));
                if idx == selected {
                    item = item.style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                list_items.push(item);
            }
            frame.render_widget(List::new(list_items).block(main_panel_block()), chunks[1]);

            let footer = Paragraph::new(vec![
                Line::from(hint.to_string()),
                Line::from(format!("Status: {status}")),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(footer, chunks[2]);
        })
        .map_err(|e| format!("failed to render list screen: {e}"))?;
    Ok(())
}

pub(crate) fn draw_setup_ui(frame: &mut Frame<'_>, view_model: &SetupMenuViewModel) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(frame.area());

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "DireClaw Setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(view_model.mode_line.clone()),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    let mut items = Vec::with_capacity(view_model.items.len());
    for (idx, label) in view_model.items.iter().enumerate() {
        let text = label.clone();
        let mut item = ListItem::new(Line::from(Span::raw(text)));
        if idx == view_model.selected {
            item = item.style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        }
        items.push(item);
    }
    let menu = List::new(items).block(main_panel_block());
    frame.render_widget(menu, chunks[1]);

    let footer = Paragraph::new(vec![
        Line::from(view_model.hint_text.clone()),
        Line::from(format!("Status: {}", view_model.status_text)),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn main_panel_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(3, 3, 2, 2))
}
