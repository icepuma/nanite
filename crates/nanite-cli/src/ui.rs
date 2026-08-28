use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem as WidgetListItem, ListState, Paragraph, Wrap,
};
use ratatui::{Frame, Terminal};
use std::io::{self, Stderr, Stdout, Write};

pub struct StatusTerminal {
    session: TerminalSession<Stderr>,
}

impl StatusTerminal {
    pub fn stderr() -> Result<Self> {
        Ok(Self {
            session: TerminalSession::stderr()?,
        })
    }

    pub fn draw<F>(&mut self, render: F) -> Result<()>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        self.session.draw(render)
    }
}

pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    let mut terminal = TerminalSession::stdout()?;
    let mut accepted = default;

    loop {
        terminal.draw(|frame| draw_confirm_prompt(frame, prompt, accepted))?;

        match read_key()? {
            key if is_cancel(&key) => return Ok(false),
            KeyEvent {
                code: KeyCode::Left,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                ..
            } => accepted = true,
            KeyEvent {
                code: KeyCode::Right,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('l'),
                ..
            } => accepted = false,
            KeyEvent {
                code: KeyCode::Char('y'),
                ..
            } => return Ok(true),
            KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => return Ok(false),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => return Ok(accepted),
            _ => {}
        }
    }
}

/// Like [`confirm`], but shows a scrollable list of items above the
/// yes/no row. Use ↑/↓ (or j/k), PageUp/PageDown, Home/End to scroll;
/// ←/→ (or h/l) toggle the focused answer; y/n answer directly;
/// Enter confirms; Esc cancels (returns Ok(false)).
pub fn confirm_with_listing(prompt: &str, items: &[String], default: bool) -> Result<bool> {
    let mut terminal = TerminalSession::stdout()?;
    let mut accepted = default;
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(0));
    }
    let len = items.len();

    loop {
        terminal.draw(|frame| {
            draw_confirm_with_listing(frame, prompt, items, &mut state, accepted);
        })?;

        match read_key()? {
            key if is_cancel(&key) => return Ok(false),
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                ..
            } if len > 0 => {
                let cur = state.selected().unwrap_or(0);
                state.select(Some(cur.saturating_sub(1)));
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                ..
            } if len > 0 => {
                let cur = state.selected().unwrap_or(0);
                state.select(Some((cur + 1).min(len - 1)));
            }
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } if len > 0 => {
                let cur = state.selected().unwrap_or(0);
                state.select(Some(cur.saturating_sub(10)));
            }
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } if len > 0 => {
                let cur = state.selected().unwrap_or(0);
                state.select(Some((cur + 10).min(len - 1)));
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } if len > 0 => {
                state.select(Some(0));
            }
            KeyEvent {
                code: KeyCode::End, ..
            } if len > 0 => {
                state.select(Some(len - 1));
            }
            KeyEvent {
                code: KeyCode::Left,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                ..
            } => accepted = true,
            KeyEvent {
                code: KeyCode::Right,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('l'),
                ..
            } => accepted = false,
            KeyEvent {
                code: KeyCode::Char('y'),
                ..
            } => return Ok(true),
            KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => return Ok(false),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => return Ok(accepted),
            _ => {}
        }
    }
}

fn draw_confirm_with_listing(
    frame: &mut Frame<'_>,
    prompt: &str,
    items: &[String],
    state: &mut ListState,
    accepted: bool,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(Line::styled(prompt, title_style())).block(panel("Confirm"));
    frame.render_widget(header, sections[0]);

    let total = items.len();
    let width = total.to_string().len();
    let list_items: Vec<WidgetListItem<'_>> = items
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            WidgetListItem::new(Line::raw(format!(
                "{:>width$}. {name}",
                idx + 1,
                width = width,
            )))
        })
        .collect();

    let list_title = format!("Repositories ({total})");
    let list = List::new(list_items)
        .block(panel(&list_title))
        .highlight_style(highlight_style())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, sections[1], state);

    let yes = if accepted { "[ Yes ]" } else { "  Yes  " };
    let no = if accepted { "  No  " } else { "[ No ]" };
    let answers = Paragraph::new(Line::from(vec![
        if accepted {
            Span::styled(yes.to_owned(), highlight_style())
        } else {
            Span::styled(yes.to_owned(), muted_style())
        },
        Span::raw("   "),
        if accepted {
            Span::styled(no.to_owned(), muted_style())
        } else {
            Span::styled(no.to_owned(), highlight_style())
        },
    ]))
    .block(panel("Choice"));
    frame.render_widget(answers, sections[2]);

    let footer = Paragraph::new(legend_line(&[
        ("↑↓/jk", "scroll"),
        ("PgUp/PgDn", "jump"),
        ("←→/hl", "switch"),
        ("y/n", "answer"),
        ("enter", "confirm"),
        ("esc", "cancel"),
    ]))
    .block(panel("Keys"));
    frame.render_widget(footer, sections[3]);
}

fn draw_confirm_prompt(frame: &mut Frame<'_>, prompt: &str, accepted: bool) {
    let popup = centered_rect(72, 11, frame.area());
    frame.render_widget(Clear, popup);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .split(popup);

    let question = Paragraph::new(Text::from(vec![
        Line::styled(prompt, title_style()),
        Line::styled(
            "Confirm the action before Nanite changes the workspace.",
            muted_style(),
        ),
        Line::styled(
            "Use arrows or direct keys for a faster decision.",
            caption_style(),
        ),
    ]))
    .block(panel("Confirm"));
    frame.render_widget(question, sections[0]);

    let yes = if accepted {
        "[ Yes ]".to_owned()
    } else {
        "  Yes  ".to_owned()
    };
    let no = if accepted {
        "  No  ".to_owned()
    } else {
        "[ No ]".to_owned()
    };
    let answers = Paragraph::new(Line::from(vec![
        if accepted {
            ratatui::text::Span::styled(yes, highlight_style())
        } else {
            ratatui::text::Span::styled(yes, muted_style())
        },
        ratatui::text::Span::raw("   "),
        if accepted {
            ratatui::text::Span::styled(no, muted_style())
        } else {
            ratatui::text::Span::styled(no, highlight_style())
        },
    ]))
    .block(panel("Choice"));
    frame.render_widget(answers, sections[1]);

    let footer = Paragraph::new(Text::from(vec![
        legend_line(&[
            ("←→", "switch"),
            ("y/n", "answer"),
            ("enter", "confirm"),
            ("esc", "cancel"),
        ]),
        Line::raw(""),
        Line::styled(
            "Default focus is shown with the highlighted choice.",
            caption_style(),
        ),
    ]))
    .block(panel("Keys"))
    .wrap(Wrap { trim: false });
    frame.render_widget(footer, sections[2]);
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(width_percent).saturating_div(100);
    let width = width.max(40).min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2)).max(5);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn read_key() -> Result<KeyEvent> {
    loop {
        match event::read().context("failed to read terminal input")? {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                return Ok(key);
            }
            _ => {}
        }
    }
}

const fn is_cancel(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc)
        || matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
        )
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(67, 81, 96)))
        .title(Line::styled(title.to_owned(), accent_style()))
}

fn accent_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 196, 87))
        .add_modifier(Modifier::BOLD)
}

fn title_style() -> Style {
    Style::default()
        .fg(Color::Rgb(145, 214, 128))
        .add_modifier(Modifier::BOLD)
}

fn muted_style() -> Style {
    Style::default().fg(Color::Rgb(144, 154, 169))
}

fn caption_style() -> Style {
    Style::default().fg(Color::Rgb(112, 122, 137))
}

fn highlight_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(97, 219, 194))
        .add_modifier(Modifier::BOLD)
}

fn legend_line(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, description)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(key_badge(key));
        spans.push(Span::raw(" "));
        spans.push(Span::styled((*description).to_owned(), muted_style()));
    }
    Line::from(spans)
}

fn key_badge(key: &str) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(255, 196, 87))
            .add_modifier(Modifier::BOLD),
    )
}

struct TerminalSession<W>
where
    W: Write,
{
    terminal: Terminal<CrosstermBackend<W>>,
    restored: bool,
}

impl TerminalSession<Stdout> {
    fn stdout() -> Result<Self> {
        let mut writer = io::stdout();
        enable_raw_mode().context("failed to enable raw mode")?;
        execute!(writer, EnterAlternateScreen, Hide).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(writer);
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }
}

impl TerminalSession<Stderr> {
    fn stderr() -> Result<Self> {
        let mut writer = io::stderr();
        enable_raw_mode().context("failed to enable raw mode")?;
        execute!(writer, EnterAlternateScreen, Hide).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(writer);
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }
}

impl<W> TerminalSession<W>
where
    W: Write,
{
    fn draw<F>(&mut self, render: F) -> Result<()>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        self.terminal
            .draw(render)
            .context("failed to render terminal UI")?;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        disable_raw_mode().context("failed to disable raw mode")?;
        execute!(self.terminal.backend_mut(), Show, LeaveAlternateScreen)
            .context("failed to leave alternate screen")?;
        self.terminal
            .show_cursor()
            .context("failed to restore cursor")?;
        self.restored = true;
        Ok(())
    }
}

impl<W> Drop for TerminalSession<W>
where
    W: Write,
{
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
