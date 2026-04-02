use crate::cli::RepoCommands;
use crate::context::ContextState;
use crate::ui::{StatusTerminal, confirm};
use crate::util::{load_registry, resolve_cli_path};
use anyhow::{Result, bail};
use camino::Utf8Path;
use nanite_git::{
    CloneProgressDisplay, SharedCloneProgressDisplay, clone_repo, import_repo, remove_repo,
    resolve_repo_remove_target, scan_workspace,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap};
use std::io::{self, IsTerminal};
use std::sync::{Arc, Mutex};

pub fn command_repo(context: &ContextState, command: RepoCommands) -> Result<()> {
    let mut registry = load_registry(&context.app_paths)?;

    match command {
        RepoCommands::Clone { remote, force } => {
            let progress = clone_progress_display(&remote)?;
            let result = clone_repo(
                context.workspace_paths.repos_root(),
                &remote,
                force,
                progress.clone(),
            );
            drop(progress);
            let record = result?;
            println!("cloned {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Import { source } => {
            let source = resolve_cli_path(&source)?;
            let record = import_repo(
                context.workspace_paths.repos_root(),
                &source,
                &context.git_binary,
            )?;
            println!("imported {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Remove { target, yes } => {
            let destination =
                resolve_repo_remove_target(context.workspace_paths.repos_root(), &target)?;
            confirm_repo_removal(&destination, yes)?;
            let removed_path = remove_repo(context.workspace_paths.repos_root(), &target)?;
            println!("removed {removed_path}");
            registry.remove_path(&removed_path);
        }
        RepoCommands::Refresh => {
            let records =
                scan_workspace(&context.git_binary, context.workspace_paths.repos_root())?;
            let count = records.len();
            for record in records {
                registry.upsert(record);
            }
            println!("refreshed {count} repositories");
        }
    }

    registry.save(&context.app_paths.registry_file())
}

fn clone_progress_display(remote: &str) -> Result<Option<SharedCloneProgressDisplay>> {
    if !io::stderr().is_terminal() {
        return Ok(None);
    }

    let display = CloneProgressTui::new(remote)?;
    Ok(Some(Arc::new(Mutex::new(display))))
}

fn confirm_repo_removal(path: &Utf8Path, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        if confirm(&format!("Remove repository at {path}?"), false)? {
            return Ok(());
        }
        bail!("aborted removal of {path}");
    }

    bail!("repo remove requires confirmation; rerun with --yes");
}

struct CloneProgressTui {
    terminal: StatusTerminal,
    remote: String,
    total: Option<usize>,
    position: usize,
    message: String,
}

impl CloneProgressTui {
    fn new(remote: &str) -> Result<Self> {
        let mut progress = Self {
            terminal: StatusTerminal::stderr()?,
            remote: remote.to_owned(),
            total: None,
            position: 0,
            message: "Preparing clone".to_owned(),
        };
        progress.render();
        Ok(progress)
    }

    fn render(&mut self) {
        let snapshot = CloneProgressSnapshot {
            remote: self.remote.clone(),
            total: self.total,
            position: self.position,
            message: self.message.clone(),
        };
        let _ = self
            .terminal
            .draw(|frame| render_clone_progress(frame, &snapshot));
    }
}

impl CloneProgressDisplay for CloneProgressTui {
    fn set_total(&mut self, total: Option<usize>) {
        self.total = total;
        self.render();
    }

    fn set_position(&mut self, position: usize) {
        self.position = position;
        self.render();
    }

    fn set_message(&mut self, message: &str) {
        message.clone_into(&mut self.message);
        self.render();
    }
}

struct CloneProgressSnapshot {
    remote: String,
    total: Option<usize>,
    position: usize,
    message: String,
}

fn render_clone_progress(frame: &mut Frame<'_>, snapshot: &CloneProgressSnapshot) {
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(4),
        ])
        .split(area);

    let header = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled("nanite repo clone", title_style()),
            Span::raw("  "),
            Span::styled(snapshot.remote.clone(), muted_style()),
        ]),
        Line::styled(
            "Cloning into the Nanite workspace with live Git progress.",
            muted_style(),
        ),
        Line::styled(
            "Nanite is streaming progress directly from the upstream clone.",
            caption_style(),
        ),
    ]))
    .block(panel("Status"));
    frame.render_widget(header, sections[0]);

    let percent = snapshot.total.and_then(|total| {
        if total == 0 {
            None
        } else {
            let percent = snapshot.position.saturating_mul(100) / total;
            Some(u16::try_from(percent.min(100)).unwrap_or(100))
        }
    });
    if let (Some(total), Some(percent)) = (snapshot.total, percent) {
        let gauge = Gauge::default()
            .block(panel("Progress"))
            .gauge_style(
                Style::default()
                    .fg(Color::Cyan)
                    .bg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .percent(percent)
            .label(format!("{}/{}", snapshot.position.min(total), total));
        frame.render_widget(gauge, sections[1]);
    } else {
        let gauge = Paragraph::new(Text::from(vec![Line::styled(
            format!("{} steps reported", snapshot.position),
            muted_style(),
        )]))
        .block(panel("Progress"));
        frame.render_widget(gauge, sections[1]);
    }

    let phase = Paragraph::new(Text::from(vec![
        Line::styled(snapshot.message.clone(), active_style()),
        Line::raw(""),
        Line::styled(
            "The terminal will return when the clone completes.",
            caption_style(),
        ),
    ]))
    .block(panel("Phase"))
    .wrap(Wrap { trim: false });
    frame.render_widget(phase, sections[2]);

    let footer = Paragraph::new(Text::from(vec![Line::styled(
        "Git progress events are streamed directly from the upstream clone operation.",
        caption_style(),
    )]))
    .block(panel("Info"))
    .wrap(Wrap { trim: false });
    frame.render_widget(footer, sections[3]);
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
