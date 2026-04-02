use crate::ui::StatusTerminal;
use nanite_core::PreparedBundle;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use std::io::{self, IsTerminal};

enum InitProgressMode {
    Tty(StatusTerminal),
    Plain,
}

#[derive(Clone, Copy)]
enum InitStepState {
    Pending,
    Active,
    Done,
    Failed,
}

struct InitStep {
    label: String,
    status: InitStepState,
    detail: Option<String>,
}

pub struct InitProgress {
    mode: InitProgressMode,
    bundle_name: String,
    template_count: usize,
    steps: Vec<InitStep>,
    inspect_index: Option<usize>,
    generate_index: Option<usize>,
    verify_index: Option<usize>,
    repair_index: Option<usize>,
    reverify_index: Option<usize>,
    write_index: usize,
}

impl InitProgress {
    pub fn new(prepared: &PreparedBundle) -> Self {
        let mut steps = vec![
            InitStep {
                label: "Select bundle".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            },
            InitStep {
                label: "Collect text inputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            },
        ];
        let mut inspect_index = None;
        let mut generate_index = None;
        let mut verify_index = None;
        let mut repair_index = None;
        let mut reverify_index = None;

        if prepared.requires_agent() {
            inspect_index = Some(steps.len());
            steps.push(InitStep {
                label: "Inspect repository context".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            generate_index = Some(steps.len());
            steps.push(InitStep {
                label: "Generate AI fragments".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            verify_index = Some(steps.len());
            steps.push(InitStep {
                label: "Verify outputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            repair_index = Some(steps.len());
            steps.push(InitStep {
                label: "Repair failing sections".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            reverify_index = Some(steps.len());
            steps.push(InitStep {
                label: "Re-verify outputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
        }

        let write_index = steps.len();
        steps.push(InitStep {
            label: "Write files".to_owned(),
            status: InitStepState::Pending,
            detail: None,
        });

        let mode = if io::stderr().is_terminal() {
            StatusTerminal::stderr()
                .map(InitProgressMode::Tty)
                .unwrap_or(InitProgressMode::Plain)
        } else {
            InitProgressMode::Plain
        };

        let mut progress = Self {
            mode,
            bundle_name: prepared.name.clone(),
            template_count: prepared.templates().len(),
            steps,
            inspect_index,
            generate_index,
            verify_index,
            repair_index,
            reverify_index,
            write_index,
        };
        progress.render();
        progress
    }

    pub const fn select_step_index() -> usize {
        0
    }

    pub const fn collect_inputs_step_index() -> usize {
        1
    }

    pub fn inspect_step_index(&self) -> usize {
        self.inspect_index.unwrap_or(self.write_index)
    }

    pub fn generate_step_index(&self) -> usize {
        self.generate_index.unwrap_or(self.write_index)
    }

    pub fn verify_step_index(&self) -> usize {
        self.verify_index.unwrap_or(self.write_index)
    }

    pub fn repair_step_index(&self) -> usize {
        self.repair_index.unwrap_or(self.write_index)
    }

    pub fn reverify_step_index(&self) -> usize {
        self.reverify_index.unwrap_or(self.write_index)
    }

    pub const fn write_step_index(&self) -> usize {
        self.write_index
    }

    pub fn mark_done(&mut self, index: usize, detail: Option<&str>) {
        self.steps[index].status = InitStepState::Done;
        self.steps[index].detail = detail.map(ToOwned::to_owned);
        self.render();
    }

    pub fn start(&mut self, index: usize, detail: Option<&str>) {
        self.steps[index].status = InitStepState::Active;
        self.steps[index].detail = detail.map(ToOwned::to_owned);
        self.render();
    }

    pub fn fail(&mut self, index: usize, detail: &str) {
        self.steps[index].status = InitStepState::Failed;
        self.steps[index].detail = Some(detail.to_owned());
        self.render();
    }

    pub fn finish_success(self) {
        drop(self);
    }

    pub fn finish_failure(self) {
        drop(self);
    }

    fn render(&mut self) {
        let fallback = self.last_milestone();
        let snapshot = InitProgressSnapshot::from(&*self);
        match &mut self.mode {
            InitProgressMode::Tty(terminal) => {
                if terminal
                    .draw(|frame| render_progress_frame(frame, &snapshot))
                    .is_err()
                {
                    eprintln!("{fallback}");
                }
            }
            InitProgressMode::Plain => {
                eprintln!("{fallback}");
            }
        }
    }

    #[cfg(test)]
    pub fn rendered(&self) -> String {
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| matches!(step.status, InitStepState::Failed))
        {
            return Self::render_step_message("failed", step);
        }
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| matches!(step.status, InitStepState::Active))
        {
            return Self::render_step_message("working", step);
        }
        if let Some(step) = self
            .steps
            .iter()
            .rfind(|step| matches!(step.status, InitStepState::Done))
        {
            return Self::render_step_message("done", step);
        }

        "waiting".to_owned()
    }

    fn last_milestone(&self) -> String {
        let step = self
            .steps
            .iter()
            .rev()
            .find(|step| !matches!(step.status, InitStepState::Pending))
            .unwrap_or(&self.steps[0]);
        let verb = match step.status {
            InitStepState::Pending => "pending",
            InitStepState::Active => "active",
            InitStepState::Done => "done",
            InitStepState::Failed => "failed",
        };
        step.detail.as_ref().map_or_else(
            || format!("{verb} {}", step.label),
            |detail| format!("{verb} {}: {detail}", step.label),
        )
    }

    fn current_step_index(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|step| matches!(step.status, InitStepState::Failed))
            .or_else(|| {
                self.steps
                    .iter()
                    .position(|step| matches!(step.status, InitStepState::Active))
            })
            .or_else(|| {
                self.steps
                    .iter()
                    .rposition(|step| matches!(step.status, InitStepState::Done))
            })
    }

    fn truncate_progress_detail(detail: &str) -> String {
        const MAX_CHARS: usize = 72;
        let compact = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut truncated = compact.chars().take(MAX_CHARS).collect::<String>();
        if compact.chars().count() > MAX_CHARS {
            truncated.push('…');
        }
        truncated
    }

    #[cfg(test)]
    fn render_step_message(prefix: &str, step: &InitStep) -> String {
        step.detail.as_ref().map_or_else(
            || format!("{prefix} {}", step.label),
            |detail| format!("{prefix} {}: {detail}", step.label),
        )
    }
}

struct InitProgressSnapshot {
    bundle_name: String,
    template_count: usize,
    current_index: Option<usize>,
    steps: Vec<InitStepSnapshot>,
}

struct InitStepSnapshot {
    label: String,
    status: InitStepState,
    detail: Option<String>,
}

impl From<&InitProgress> for InitProgressSnapshot {
    fn from(progress: &InitProgress) -> Self {
        Self {
            bundle_name: progress.bundle_name.clone(),
            template_count: progress.template_count,
            current_index: progress.current_step_index(),
            steps: progress
                .steps
                .iter()
                .map(|step| InitStepSnapshot {
                    label: step.label.clone(),
                    status: step.status,
                    detail: step.detail.clone(),
                })
                .collect(),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn render_progress_frame(frame: &mut Frame<'_>, snapshot: &InitProgressSnapshot) {
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(sections[1]);

    let header = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled("nanite init", title_style()),
            Span::raw("  "),
            Span::styled(format!("bundle: {}", snapshot.bundle_name), muted_style()),
        ]),
        Line::styled(
            format!(
                "Preparing {} file(s) in the current repository",
                snapshot.template_count
            ),
            muted_style(),
        ),
        Line::styled(
            "Track bundle selection, AI generation, verification, and file writes.",
            caption_style(),
        ),
    ]))
    .block(panel("Status"));
    frame.render_widget(header, sections[0]);

    let items = snapshot
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let icon = match step.status {
                InitStepState::Pending => "○",
                InitStepState::Active => "▶",
                InitStepState::Done => "✓",
                InitStepState::Failed => "×",
            };
            let icon_style = match step.status {
                InitStepState::Pending => muted_style(),
                InitStepState::Active => active_style(),
                InitStepState::Done => done_style(),
                InitStepState::Failed => failed_style(),
            };
            let detail = step
                .detail
                .as_deref()
                .map(InitProgress::truncate_progress_detail)
                .filter(|detail| !detail.is_empty());
            let line = detail.map_or_else(
                || {
                    Line::from(vec![
                        Span::styled(icon, icon_style),
                        Span::raw(" "),
                        Span::raw(step.label.clone()),
                    ])
                },
                |detail| {
                    Line::from(vec![
                        Span::styled(icon, icon_style),
                        Span::raw(" "),
                        Span::raw(step.label.clone()),
                        Span::styled(format!("  {detail}"), muted_style()),
                    ])
                },
            );
            let mut item = ListItem::new(line);
            if Some(index) == snapshot.current_index {
                item = item.style(Style::default().add_modifier(Modifier::BOLD));
            }
            item
        })
        .collect::<Vec<_>>();
    let list = List::new(items).block(panel("Steps"));
    frame.render_widget(list, body[0]);

    let detail = snapshot
        .current_index
        .and_then(|index| snapshot.steps.get(index));
    let detail_lines = detail.map_or_else(
        || {
            vec![
                Line::styled("Waiting for work", muted_style()),
                Line::raw(""),
                Line::styled("The selected bundle is ready to render.", muted_style()),
            ]
        },
        |step| {
            let status = match step.status {
                InitStepState::Pending => "Queued",
                InitStepState::Active => "Working",
                InitStepState::Done => "Done",
                InitStepState::Failed => "Failed",
            };
            let status_style = match step.status {
                InitStepState::Pending => muted_style(),
                InitStepState::Active => active_style(),
                InitStepState::Done => done_style(),
                InitStepState::Failed => failed_style(),
            };
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(status, status_style),
                    Span::raw("  "),
                    Span::raw(step.label.clone()),
                ]),
                Line::raw(""),
            ];
            if let Some(detail) = &step.detail {
                lines.push(Line::raw(detail.clone()));
                lines.push(Line::raw(""));
            }
            lines.push(Line::styled(
                "Nanite will restore the terminal when this run completes.",
                muted_style(),
            ));
            lines
        },
    );
    let detail = Paragraph::new(Text::from(detail_lines))
        .block(panel("Current step"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, body[1]);

    let footer = Paragraph::new(Text::from(vec![Line::styled(
        "This live view stays open until the current render completes or fails.",
        caption_style(),
    )]))
    .block(panel("Info"));
    frame.render_widget(footer, sections[2]);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(67, 81, 96)))
        .title(Line::styled(
            title.to_owned(),
            Style::default()
                .fg(Color::Rgb(255, 196, 87))
                .add_modifier(Modifier::BOLD),
        ))
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

fn active_style() -> Style {
    Style::default()
        .fg(Color::Rgb(97, 219, 194))
        .add_modifier(Modifier::BOLD)
}

fn done_style() -> Style {
    Style::default()
        .fg(Color::Rgb(145, 214, 128))
        .add_modifier(Modifier::BOLD)
}

fn failed_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 120, 120))
        .add_modifier(Modifier::BOLD)
}
