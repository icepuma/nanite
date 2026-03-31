use crate::ui::inquire_render_config;
use anyhow::{Context, Result, anyhow, bail};
use inquire::{Select, Text};
use nanite_core::{Prompter, TextPlaceholder};
use std::io::{Read, Write};

pub(super) trait InitPrompter: Prompter {
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize>;
}

pub(super) struct InquirePrompter;

impl InitPrompter for InquirePrompter {
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize> {
        if options.is_empty() {
            bail!("{prompt} has no options");
        }

        let selected = Select::new(prompt, options.to_vec())
            .with_render_config(inquire_render_config())
            .prompt()
            .with_context(|| format!("failed to choose {prompt}"))?;
        options
            .iter()
            .position(|option| option == &selected)
            .ok_or_else(|| anyhow!("selected option `{selected}` was not in the prompt list"))
    }
}

impl Prompter for InquirePrompter {
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        Text::new(&placeholder.prompt)
            .with_render_config(inquire_render_config())
            .prompt()
            .with_context(|| format!("failed to capture {}", placeholder.prompt))
    }
}

pub(super) struct IoPrompter<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> IoPrompter<R, W> {
    pub(super) const fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn read_line(&mut self) -> Result<String> {
        let mut buffer = String::new();
        let mut byte = [0_u8; 1];
        loop {
            let read = self.reader.read(&mut byte)?;
            if read == 0 || byte[0] == b'\n' {
                break;
            }
            buffer.push(char::from(byte[0]));
        }

        Ok(buffer)
    }
}

impl<R, W> InitPrompter for IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize> {
        if options.is_empty() {
            bail!("{prompt} has no options");
        }

        loop {
            writeln!(self.writer, "{prompt}:")?;
            for (index, option) in options.iter().enumerate() {
                writeln!(self.writer, "  {}. {}", index + 1, option)?;
            }
            write!(self.writer, "Choice [1]: ")?;
            self.writer.flush()?;

            let response = self.read_line()?;
            let trimmed = response.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            if let Ok(choice) = trimmed.parse::<usize>()
                && (1..=options.len()).contains(&choice)
            {
                return Ok(choice - 1);
            }
            if let Some(index) = options.iter().position(|option| option == trimmed) {
                return Ok(index);
            }

            writeln!(
                self.writer,
                "Enter a number between 1 and {}.",
                options.len()
            )?;
        }
    }
}

impl<R, W> Prompter for IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        write!(self.writer, "{}: ", placeholder.prompt)?;
        self.writer.flush()?;
        Ok(self.read_line()?.trim().to_owned())
    }
}
