use indicatif::{ProgressBar, ProgressStyle};
use nanite_core::PreparedBundle;
use std::io::{self, IsTerminal};
use std::time::Duration;

enum InitProgressMode {
    Tty(ProgressBar),
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
            let bar = ProgressBar::new(steps.len() as u64);
            let style = ProgressStyle::with_template(
                "{spinner:.cyan} {prefix:.bold.dim} {wide_bar:.cyan/blue} {msg}",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▉▊▋▌▍▎▏ ")
            .tick_strings(&["⠋", "⠙", "⠸", "⠴", "⠦", "⠇"]);
            bar.set_style(style);
            bar.enable_steady_tick(Duration::from_millis(100));
            InitProgressMode::Tty(bar)
        } else {
            InitProgressMode::Plain
        };

        let progress = Self {
            mode,
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
        match self.mode {
            InitProgressMode::Tty(progress) => progress.finish_and_clear(),
            InitProgressMode::Plain => {}
        }
    }

    pub fn finish_failure(self) {
        let rendered = self.rendered();
        match self.mode {
            InitProgressMode::Tty(progress) => progress.abandon_with_message(rendered),
            InitProgressMode::Plain => {}
        }
    }

    fn render(&self) {
        match &self.mode {
            InitProgressMode::Tty(progress) => {
                progress.set_position(self.tty_position() as u64);
                progress.set_prefix(self.tty_prefix());
                progress.set_message(self.tty_message());
            }
            InitProgressMode::Plain => {
                eprintln!("{}", self.last_milestone());
            }
        }
    }

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

    fn completed_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step.status, InitStepState::Done))
            .count()
    }

    fn tty_position(&self) -> usize {
        let completed = self.completed_steps();
        if self
            .steps
            .iter()
            .any(|step| matches!(step.status, InitStepState::Active))
        {
            (completed + 1).min(self.steps.len())
        } else {
            completed
        }
    }

    fn tty_prefix(&self) -> String {
        let index = self
            .current_step_index()
            .map_or(1, |index| index.saturating_add(1));
        format!("step {index}/{}", self.steps.len())
    }

    fn tty_message(&self) -> String {
        let Some(index) = self.current_step_index() else {
            return "Waiting for work".to_owned();
        };
        let step = &self.steps[index];
        let status = match step.status {
            InitStepState::Pending => "Queued",
            InitStepState::Active => "Working",
            InitStepState::Done => "Done",
            InitStepState::Failed => "Failed",
        };
        let detail = step
            .detail
            .as_deref()
            .map(Self::truncate_progress_detail)
            .filter(|detail| !detail.is_empty());
        detail.map_or_else(
            || format!("{status} · {}", step.label),
            |detail| format!("{status} · {} · {detail}", step.label),
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

    fn render_step_message(prefix: &str, step: &InitStep) -> String {
        step.detail.as_ref().map_or_else(
            || format!("{prefix} {}", step.label),
            |detail| format!("{prefix} {}: {detail}", step.label),
        )
    }
}
