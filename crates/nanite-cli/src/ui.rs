use anyhow::{Context, Result, bail};
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
use std::collections::BTreeSet;
use std::io::{self, Stderr, Stdout, Write};

pub const GITIGNORE_UPSTREAM_REPO: &str = "github/gitignore";

pub struct BrowserItem<T>
where
    T: Copy,
{
    pub(crate) value: T,
    pub(crate) label: String,
    pub(crate) caption: Option<String>,
    pub(crate) search_terms: String,
    pub(crate) detail_lines: Vec<String>,
}

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

pub fn choose(prompt: &str, options: &[String]) -> Result<usize> {
    if options.is_empty() {
        bail!("{prompt} has no options");
    }

    let items = options
        .iter()
        .enumerate()
        .map(|(index, option)| BrowserItem {
            value: index,
            label: option.clone(),
            caption: None,
            search_terms: option.clone(),
            detail_lines: vec![
                format!("Selection: {option}"),
                String::new(),
                "Use the search box to narrow the list, then press Enter to choose.".to_owned(),
            ],
        })
        .collect();

    choose_from_browser(
        prompt,
        "Browse with the arrow keys or j/k. Type to filter, Enter to choose.",
        items,
    )
}

pub fn multi_select<T>(title: &str, subtitle: &str, items: Vec<BrowserItem<T>>) -> Result<Vec<T>>
where
    T: Copy,
{
    if items.is_empty() {
        bail!("{title} has no options");
    }

    let mut terminal = TerminalSession::stdout()?;
    let mut state = BrowserState::new(items);
    let mut selected = BTreeSet::new();
    loop {
        terminal.draw(|frame| {
            draw_browser(frame, title, subtitle, &state, Some(&selected), true);
        })?;

        match read_key()? {
            key if is_cancel(&key) => bail!("cancelled {title}"),
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                ..
            } => state.move_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                ..
            } => state.move_down(),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => state.page_up(8),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => state.page_down(8),
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => state.move_home(),
            KeyEvent {
                code: KeyCode::End, ..
            } => state.move_end(),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => state.backspace_filter(),
            KeyEvent {
                code: KeyCode::Char(' '),
                ..
            } => {
                if let Some(index) = state.current_global_index() {
                    if !selected.insert(index) {
                        selected.remove(&index);
                    }
                    state.set_notice(None);
                }
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if selected.is_empty() {
                    state.set_notice(Some("Select at least one template".to_owned()));
                    continue;
                }

                return Ok(selected
                    .into_iter()
                    .map(|index| state.items[index].value)
                    .collect());
            }
            KeyEvent {
                code: KeyCode::Char(character),
                modifiers,
                ..
            } if accepts_text_input(modifiers) => state.push_filter(character),
            _ => {}
        }
    }
}

pub fn prompt_text(prompt: &str) -> Result<String> {
    let mut terminal = TerminalSession::stdout()?;
    let mut value = String::new();

    loop {
        terminal.draw(|frame| draw_text_prompt(frame, prompt, &value))?;

        match read_key()? {
            key if is_cancel(&key) => bail!("cancelled {prompt}"),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                value.pop();
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => return Ok(value),
            KeyEvent {
                code: KeyCode::Char(character),
                modifiers,
                ..
            } if accepts_text_input(modifiers) => value.push(character),
            _ => {}
        }
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

fn choose_from_browser<T>(title: &str, subtitle: &str, items: Vec<BrowserItem<T>>) -> Result<T>
where
    T: Copy,
{
    let mut terminal = TerminalSession::stdout()?;
    let mut state = BrowserState::new(items);
    loop {
        terminal.draw(|frame| draw_browser(frame, title, subtitle, &state, None, false))?;

        match read_key()? {
            key if is_cancel(&key) => bail!("cancelled {title}"),
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                ..
            } => state.move_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                ..
            } => state.move_down(),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => state.page_up(8),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => state.page_down(8),
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => state.move_home(),
            KeyEvent {
                code: KeyCode::End, ..
            } => state.move_end(),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => state.backspace_filter(),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if let Some(index) = state.current_global_index() {
                    return Ok(state.items[index].value);
                }
                state.set_notice(Some("No matching option is selected".to_owned()));
            }
            KeyEvent {
                code: KeyCode::Char(character),
                modifiers,
                ..
            } if accepts_text_input(modifiers) => state.push_filter(character),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_lines)]
fn draw_browser<T>(
    frame: &mut Frame<'_>,
    title: &str,
    subtitle: &str,
    state: &BrowserState<T>,
    selected: Option<&BTreeSet<usize>>,
    show_checkboxes: bool,
) where
    T: Copy,
{
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(4),
        ])
        .split(area);
    let body_sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(sections[2]);
    let selected_count = selected.map_or(0, BTreeSet::len);
    let summary = if show_checkboxes {
        format!(
            "{} shown · {} selected · bundled from {GITIGNORE_UPSTREAM_REPO}",
            state.filtered_indices.len(),
            selected_count
        )
    } else {
        format!(
            "{} shown · {} total options",
            state.filtered_indices.len(),
            state.items.len()
        )
    };

    let header = Paragraph::new(Text::from(vec![
        Line::styled(title, title_style()),
        Line::styled(subtitle, muted_style()),
        Line::styled(summary, caption_style()),
    ]))
    .block(panel("Nanite"));
    frame.render_widget(header, sections[0]);

    let search_text = if state.filter.is_empty() {
        Text::from(vec![Line::styled(
            "Type to filter by name, id, group, or source path",
            muted_style(),
        )])
    } else {
        Text::from(vec![Line::from(vec![
            Span::styled("filter", label_style()),
            Span::raw("  "),
            Span::styled(state.filter.as_str(), Style::default().fg(Color::White)),
        ])])
    };
    let search = Paragraph::new(search_text).block(panel("Search"));
    frame.render_widget(search, sections[1]);

    let list_title = format!("Options ({})", state.filtered_indices.len());
    let list_block = panel(&list_title);
    if state.filtered_indices.is_empty() {
        let empty = Paragraph::new(Text::from(vec![
            Line::styled("No matches", muted_style()),
            Line::raw(""),
            Line::styled(
                "Adjust the search text to see more templates.",
                muted_style(),
            ),
        ]))
        .block(list_block)
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, body_sections[0]);
    } else {
        let items = state
            .filtered_indices
            .iter()
            .map(|index| {
                let item = &state.items[*index];
                let marker = if show_checkboxes {
                    if selected.is_some_and(|selected| selected.contains(index)) {
                        "[x]"
                    } else {
                        "[ ]"
                    }
                } else {
                    "›"
                };

                let mut spans = vec![Span::styled(marker, accent_style()), Span::raw(" ")];
                spans.push(Span::styled(
                    item.label.clone(),
                    Style::default().fg(Color::White),
                ));
                if let Some(caption) = &item.caption {
                    spans.push(Span::styled(format!("  {caption}"), muted_style()));
                }

                WidgetListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        list_state.select(Some(state.cursor));
        let list = List::new(items)
            .block(list_block)
            .highlight_style(highlight_style())
            .highlight_symbol("");
        frame.render_stateful_widget(list, body_sections[0], &mut list_state);
    }

    let detail = state.current_item().map_or_else(
        || {
            Text::from(vec![
                Line::styled("No selection", muted_style()),
                Line::raw(""),
                Line::styled("Move the cursor to inspect an entry.", muted_style()),
            ])
        },
        |item| {
            Text::from(
                item.detail_lines
                    .iter()
                    .cloned()
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
        },
    );
    let detail = Paragraph::new(detail)
        .block(panel("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, body_sections[1]);

    let footer_line = state.notice.as_deref().map_or_else(
        || {
            if show_checkboxes {
                legend_line(&[
                    ("↑↓", "move"),
                    ("space", "toggle"),
                    ("enter", "confirm"),
                    ("esc", "cancel"),
                ])
            } else {
                legend_line(&[("↑↓", "move"), ("enter", "choose"), ("esc", "cancel")])
            }
        },
        |message| Line::styled(message.to_owned(), warning_style()),
    );
    let status_line = Line::styled(
        format!(
            "visible {} of {}",
            state.filtered_indices.len(),
            state.items.len()
        ),
        caption_style(),
    );
    let footer = Paragraph::new(Text::from(vec![footer_line, Line::raw(""), status_line]))
        .block(panel("Keys"))
        .wrap(Wrap { trim: false });
    frame.render_widget(footer, sections[3]);
}

fn draw_text_prompt(frame: &mut Frame<'_>, prompt: &str, value: &str) {
    let popup = centered_rect(88, 11, frame.area());
    frame.render_widget(Clear, popup);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .split(popup);

    let title = Paragraph::new(Text::from(vec![
        Line::styled(prompt, title_style()),
        Line::styled(
            "Fill the prompt and confirm when it looks right.",
            muted_style(),
        ),
        Line::styled(
            "Nanite will use this value immediately in the template.",
            caption_style(),
        ),
    ]))
    .block(panel("Input"));
    frame.render_widget(title, sections[0]);

    let input_text = if value.is_empty() {
        Text::from(vec![Line::styled("Start typing…", muted_style())])
    } else {
        Text::from(vec![Line::styled(value, Style::default().fg(Color::White))])
    };
    let input = Paragraph::new(input_text).block(panel("Value"));
    frame.render_widget(input, sections[1]);

    let footer = Paragraph::new(Text::from(vec![
        legend_line(&[("type", "edit"), ("enter", "confirm"), ("esc", "cancel")]),
        Line::raw(""),
        Line::styled(
            "Free-form text input for the current template field.",
            caption_style(),
        ),
    ]))
    .block(panel("Keys"))
    .wrap(Wrap { trim: false });
    frame.render_widget(footer, sections[2]);
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

fn accepts_text_input(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

#[allow(clippy::missing_const_for_fn)]
fn is_cancel(key: &KeyEvent) -> bool {
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

fn label_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 196, 87))
        .add_modifier(Modifier::BOLD)
}

fn warning_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 214, 102))
        .add_modifier(Modifier::BOLD)
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

struct BrowserState<T>
where
    T: Copy,
{
    items: Vec<BrowserItem<T>>,
    filter: String,
    filtered_indices: Vec<usize>,
    cursor: usize,
    notice: Option<String>,
}

#[allow(clippy::missing_const_for_fn)]
impl<T> BrowserState<T>
where
    T: Copy,
{
    fn new(items: Vec<BrowserItem<T>>) -> Self {
        let mut state = Self {
            items,
            filter: String::new(),
            filtered_indices: Vec::new(),
            cursor: 0,
            notice: None,
        };
        state.refresh_matches();
        state
    }

    fn push_filter(&mut self, character: char) {
        self.filter.push(character);
        self.refresh_matches();
    }

    fn backspace_filter(&mut self) {
        self.filter.pop();
        self.refresh_matches();
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.cursor + 1 < self.filtered_indices.len() {
            self.cursor += 1;
        }
    }

    fn page_up(&mut self, amount: usize) {
        self.cursor = self.cursor.saturating_sub(amount);
    }

    fn page_down(&mut self, amount: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        self.cursor = (self.cursor + amount).min(self.filtered_indices.len() - 1);
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.cursor = self.filtered_indices.len() - 1;
        }
    }

    fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    fn current_global_index(&self) -> Option<usize> {
        self.filtered_indices.get(self.cursor).copied()
    }

    fn current_item(&self) -> Option<&BrowserItem<T>> {
        self.current_global_index()
            .and_then(|index| self.items.get(index))
    }

    fn refresh_matches(&mut self) {
        let filter = normalize(&self.filter);
        self.filtered_indices = self
            .items
            .iter()
            .enumerate()
            .filter(|(_index, item)| {
                filter.is_empty() || normalize(&item.search_terms).contains(&filter)
            })
            .map(|(index, _item)| index)
            .collect();
        if self.filtered_indices.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(self.filtered_indices.len() - 1);
        }
        self.notice = None;
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
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
