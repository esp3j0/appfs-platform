use crossterm::cursor::{MoveTo, MoveToColumn};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType, ScrollUp};
use runtime::{
    InputEnvelope, InputSource, PendingInput, PendingInputDelivery, SharedPendingInputQueue,
};
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMode {
    IdlePrompt,
    RunningGuidance,
    PermissionPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    SubmittedLine(String),
    Cancel,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCommand {
    SetMode(TerminalMode),
    RenderPrompt,
    SetCompletions(Vec<String>),
    SetStatus(String),
    ClearStatus,
    WriteOutput(String),
    AskPermission(PermissionPromptView),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPromptView {
    pub tool_name: String,
    pub current_mode: String,
    pub required_mode: String,
    pub reason: Option<String>,
    pub input: String,
}

pub struct PermissionPromptTicket {
    pub view: PermissionPromptView,
    pub response_tx: Sender<runtime::PermissionPromptDecision>,
}

pub struct TerminalControllerHandle {
    event_rx: Receiver<TerminalEvent>,
    command_tx: Sender<TerminalCommand>,
    join_handle: Option<JoinHandle<()>>,
}

impl TerminalControllerHandle {
    pub fn start(
        shared_queue: SharedPendingInputQueue,
        permission_rx: Receiver<PermissionPromptTicket>,
    ) -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::other(
                "running input requires an interactive terminal",
            ));
        }

        let (event_tx, event_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            let startup = terminal_controller_loop(
                shared_queue,
                permission_rx,
                event_tx,
                command_rx,
                startup_tx,
            );
            if let Err(error) = startup {
                eprintln!("running input terminal controller failed: {error}");
            }
        });

        match startup_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                event_rx,
                command_tx,
                join_handle: Some(join_handle),
            }),
            Ok(Err(message)) => {
                let _ = join_handle.join();
                Err(io::Error::other(message))
            }
            Err(error) => {
                let _ = join_handle.join();
                Err(io::Error::other(format!(
                    "terminal controller did not start: {error}"
                )))
            }
        }
    }

    pub fn send(&self, command: TerminalCommand) -> io::Result<()> {
        self.command_tx
            .send(command)
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error))
    }

    pub fn command_sender(&self) -> Sender<TerminalCommand> {
        self.command_tx.clone()
    }

    pub fn recv(&self) -> Result<TerminalEvent, mpsc::RecvError> {
        self.event_rx.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<TerminalEvent, RecvTimeoutError> {
        self.event_rx.recv_timeout(timeout)
    }

    pub fn shutdown(mut self) {
        let _ = self.command_tx.send(TerminalCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for TerminalControllerHandle {
    fn drop(&mut self) {
        let _ = self.command_tx.send(TerminalCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode().map_err(io::Error::other)?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = writeln!(io::stdout());
    }
}

fn terminal_controller_loop(
    shared_queue: SharedPendingInputQueue,
    permission_rx: Receiver<PermissionPromptTicket>,
    event_tx: Sender<TerminalEvent>,
    command_rx: Receiver<TerminalCommand>,
    startup_tx: Sender<Result<(), String>>,
) -> io::Result<()> {
    let _raw_mode = match RawModeGuard::enter() {
        Ok(guard) => {
            let _ = startup_tx.send(Ok(()));
            guard
        }
        Err(error) => {
            let _ = startup_tx.send(Err(error.to_string()));
            return Err(error);
        }
    };
    let mut state = TerminalControllerState::new(shared_queue, permission_rx, event_tx);
    state.render_prompt()?;

    loop {
        while let Ok(command) = command_rx.try_recv() {
            if state.handle_command(command)? {
                return Ok(());
            }
        }
        state.poll_permission_request()?;
        if event::poll(Duration::from_millis(50)).map_err(io::Error::other)? {
            match event::read().map_err(io::Error::other)? {
                Event::Key(key) => {
                    if state.handle_key(key)? {
                        return Ok(());
                    }
                }
                Event::Resize(_, _) => state.render_prompt()?,
                _ => {}
            }
        }
    }
}

struct TerminalControllerState {
    mode: TerminalMode,
    previous_mode: TerminalMode,
    status_line: Option<String>,
    line: String,
    completions: Vec<String>,
    permission_ticket: Option<PermissionPromptTicket>,
    shared_queue: SharedPendingInputQueue,
    permission_rx: Receiver<PermissionPromptTicket>,
    event_tx: Sender<TerminalEvent>,
}

impl TerminalControllerState {
    fn new(
        shared_queue: SharedPendingInputQueue,
        permission_rx: Receiver<PermissionPromptTicket>,
        event_tx: Sender<TerminalEvent>,
    ) -> Self {
        Self {
            mode: TerminalMode::IdlePrompt,
            previous_mode: TerminalMode::IdlePrompt,
            status_line: None,
            line: String::new(),
            completions: Vec::new(),
            permission_ticket: None,
            shared_queue,
            permission_rx,
            event_tx,
        }
    }

    fn handle_command(&mut self, command: TerminalCommand) -> io::Result<bool> {
        match command {
            TerminalCommand::SetMode(mode) => {
                let old_footer_height = self.footer_height();
                if self.set_mode(mode) {
                    self.line.clear();
                    self.preserve_output_for_footer_resize(
                        old_footer_height,
                        self.footer_height(),
                    )?;
                    self.render_footer()?;
                }
                Ok(false)
            }
            TerminalCommand::RenderPrompt => {
                self.render_footer()?;
                Ok(false)
            }
            TerminalCommand::SetCompletions(completions) => {
                self.completions = completions;
                Ok(false)
            }
            TerminalCommand::SetStatus(status) => {
                self.status_line = Some(status);
                self.render_footer()?;
                Ok(false)
            }
            TerminalCommand::ClearStatus => {
                self.status_line = None;
                self.render_footer()?;
                Ok(false)
            }
            TerminalCommand::WriteOutput(text) => {
                self.write_output(&text)?;
                Ok(false)
            }
            TerminalCommand::AskPermission(view) => {
                let (response_tx, _response_rx) = mpsc::channel();
                self.enter_permission_prompt(PermissionPromptTicket { view, response_tx })?;
                Ok(false)
            }
            TerminalCommand::Shutdown => Ok(true),
        }
    }

    fn set_mode(&mut self, mode: TerminalMode) -> bool {
        if self.mode != TerminalMode::PermissionPrompt {
            if self.mode == mode {
                return false;
            }
            self.mode = mode;
        } else {
            if self.previous_mode == mode {
                return false;
            }
            self.previous_mode = mode;
        }
        true
    }

    fn poll_permission_request(&mut self) -> io::Result<()> {
        if self.permission_ticket.is_some() {
            return Ok(());
        }
        match self.permission_rx.try_recv() {
            Ok(ticket) => self.enter_permission_prompt(ticket),
            Err(mpsc::TryRecvError::Empty) => Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => Ok(()),
        }
    }

    fn enter_permission_prompt(&mut self, ticket: PermissionPromptTicket) -> io::Result<()> {
        let old_footer_height = self.footer_height();
        if self.mode != TerminalMode::PermissionPrompt {
            self.previous_mode = self.mode;
        }
        self.mode = TerminalMode::PermissionPrompt;
        self.permission_ticket = Some(ticket);
        self.line.clear();
        self.preserve_output_for_footer_resize(old_footer_height, self.footer_height())?;
        self.render_footer()
    }

    fn handle_key(&mut self, key: KeyEvent) -> io::Result<bool> {
        if key.kind == KeyEventKind::Release {
            return Ok(false);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return self.handle_cancel_key();
        }

        match self.mode {
            TerminalMode::PermissionPrompt => self.handle_permission_key(key),
            TerminalMode::IdlePrompt | TerminalMode::RunningGuidance => self.handle_line_key(key),
        }
    }

    fn handle_line_key(&mut self, key: KeyEvent) -> io::Result<bool> {
        match key.code {
            KeyCode::Enter => {
                let submitted = std::mem::take(&mut self.line);
                match self.mode {
                    TerminalMode::IdlePrompt => {
                        if !submitted.trim().is_empty() {
                            self.write_output(&format!("> {submitted}\n"))?;
                        } else {
                            self.render_footer()?;
                        }
                        let _ = self.event_tx.send(TerminalEvent::SubmittedLine(submitted));
                    }
                    TerminalMode::RunningGuidance => {
                        if let Some(input) = pending_input_from_running_line(&submitted) {
                            self.write_output(&format!("{}\n", render_running_input_echo(&input)))?;
                            self.shared_queue.push(input);
                            self.status_line =
                                Some("Queued for the next model boundary.".to_string());
                        } else {
                            self.status_line = Some(self.running_status_line());
                        }
                        self.render_footer()?;
                    }
                    TerminalMode::PermissionPrompt => {}
                }
                Ok(false)
            }
            KeyCode::Backspace => {
                if self.line.pop().is_some() {
                    self.render_footer()?;
                }
                Ok(false)
            }
            KeyCode::Tab => {
                self.render_matching_completions()?;
                Ok(false)
            }
            KeyCode::Esc => {
                self.line.clear();
                self.render_footer()?;
                Ok(false)
            }
            KeyCode::Char(value) => {
                self.line.push(value);
                self.render_footer()?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_permission_key(&mut self, key: KeyEvent) -> io::Result<bool> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.finish_permission(runtime::PermissionPromptDecision::Allow)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.finish_permission(runtime::PermissionPromptDecision::Deny {
                    reason: "tool denied by user approval prompt".to_string(),
                })
            }
            KeyCode::Enter => {
                let normalized = self.line.trim().to_ascii_lowercase();
                let decision = if matches!(normalized.as_str(), "y" | "yes") {
                    runtime::PermissionPromptDecision::Allow
                } else {
                    runtime::PermissionPromptDecision::Deny {
                        reason: "tool denied by user approval prompt".to_string(),
                    }
                };
                self.finish_permission(decision)
            }
            KeyCode::Backspace => {
                if self.line.pop().is_some() {
                    self.render_footer()?;
                }
                Ok(false)
            }
            KeyCode::Char(value) => {
                self.line.push(value);
                self.render_footer()?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_cancel_key(&mut self) -> io::Result<bool> {
        match self.mode {
            TerminalMode::IdlePrompt => {
                if self.line.is_empty() {
                    let _ = self.event_tx.send(TerminalEvent::Exit);
                    Ok(true)
                } else {
                    self.line.clear();
                    let _ = self.event_tx.send(TerminalEvent::Cancel);
                    self.status_line = None;
                    self.render_footer()?;
                    Ok(false)
                }
            }
            TerminalMode::RunningGuidance => {
                self.line.clear();
                self.status_line = Some("Guidance cancelled.".to_string());
                self.render_footer()?;
                Ok(false)
            }
            TerminalMode::PermissionPrompt => {
                self.finish_permission(runtime::PermissionPromptDecision::Deny {
                    reason: "tool denied by user approval prompt".to_string(),
                })
            }
        }
    }

    fn finish_permission(
        &mut self,
        decision: runtime::PermissionPromptDecision,
    ) -> io::Result<bool> {
        if let Some(ticket) = self.permission_ticket.take() {
            let _ = ticket.response_tx.send(decision);
        }
        self.mode = self.previous_mode;
        self.line.clear();
        self.render_footer()?;
        Ok(false)
    }

    fn render_prompt(&self) -> io::Result<()> {
        self.render_footer()
    }

    fn write_output(&self, text: &str) -> io::Result<()> {
        let footer_start = self.footer_start_row()?;
        let mut rendered = text.to_string();
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        let spacer_lines = footer_spacer_lines(&rendered, self.footer_height());
        let mut stdout = io::stdout();
        queue!(
            stdout,
            MoveTo(0, footer_start),
            Clear(ClearType::FromCursorDown),
            Print(&rendered)
        )?;
        if spacer_lines > 0 {
            queue!(stdout, ScrollUp(spacer_lines))?;
        }
        stdout.flush()?;
        self.render_footer()
    }

    fn preserve_output_for_footer_resize(
        &self,
        old_footer_height: u16,
        new_footer_height: u16,
    ) -> io::Result<()> {
        let growth = footer_growth_lines(old_footer_height, new_footer_height);
        if growth == 0 {
            return Ok(());
        }

        let mut stdout = io::stdout();
        queue!(stdout, ScrollUp(growth))?;
        stdout.flush()
    }

    fn render_matching_completions(&self) -> io::Result<()> {
        if !self.line.starts_with('/') {
            return Ok(());
        }
        let matches = self
            .completions
            .iter()
            .filter(|candidate| candidate.starts_with(&self.line))
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Ok(());
        }
        self.write_output(&format!("{}\n", matches.join("  ")))
    }

    fn render_footer(&self) -> io::Result<()> {
        let (_, rows) = size().map_err(io::Error::other)?;
        let footer_start = rows.saturating_sub(self.footer_height());
        let mut stdout = io::stdout();
        queue!(
            stdout,
            MoveTo(0, footer_start),
            Clear(ClearType::FromCursorDown)
        )?;

        match self.mode {
            TerminalMode::IdlePrompt => {
                queue!(
                    stdout,
                    MoveTo(0, footer_start),
                    Print(render_input_line("> ", &self.line, size()?.0))
                )?;
            }
            TerminalMode::RunningGuidance => {
                queue!(
                    stdout,
                    MoveTo(0, footer_start),
                    Print(truncate_to_width(&self.running_status_line(), size()?.0)),
                    MoveTo(0, footer_start.saturating_add(1)),
                    Print(render_input_line("(guidance)> ", &self.line, size()?.0))
                )?;
            }
            TerminalMode::PermissionPrompt => {
                for (index, line) in self.permission_prompt_lines(size()?.0).iter().enumerate() {
                    queue!(
                        stdout,
                        MoveTo(0, footer_start.saturating_add(index as u16)),
                        Print(line)
                    )?;
                }
            }
        }

        stdout.flush()
    }

    fn running_status_line(&self) -> String {
        self.status_line
            .clone()
            .unwrap_or_else(|| "Running. Type and press Enter to queue guidance.".to_string())
    }

    fn permission_prompt_lines(&self, width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(ticket) = self.permission_ticket.as_ref() {
            lines.push(truncate_to_width("Permission approval required", width));
            lines.push(truncate_to_width(
                &format!("  Tool             {}", ticket.view.tool_name),
                width,
            ));
            lines.push(truncate_to_width(
                &format!("  Current mode     {}", ticket.view.current_mode),
                width,
            ));
            lines.push(truncate_to_width(
                &format!("  Required mode    {}", ticket.view.required_mode),
                width,
            ));
            if let Some(reason) = &ticket.view.reason {
                lines.push(truncate_to_width(
                    &format!("  Reason           {reason}"),
                    width,
                ));
            }
            lines.push(truncate_to_width(
                &format!("  Input            {}", ticket.view.input),
                width,
            ));
            lines.push(render_input_line(
                "Approve this tool call? [y/N]: ",
                &self.line,
                width,
            ));
        }
        lines
    }

    fn footer_height(&self) -> u16 {
        match self.mode {
            TerminalMode::IdlePrompt => 1,
            TerminalMode::RunningGuidance => 2,
            TerminalMode::PermissionPrompt => self.permission_ticket.as_ref().map_or(1, |ticket| {
                if ticket.view.reason.is_some() {
                    7
                } else {
                    6
                }
            }),
        }
    }

    fn footer_start_row(&self) -> io::Result<u16> {
        let (_, rows) = size().map_err(io::Error::other)?;
        Ok(rows.saturating_sub(self.footer_height()))
    }
}

fn backspace_sequence_for_char(ch: char) -> String {
    let width = UnicodeWidthChar::width(ch).unwrap_or(0);
    if width == 0 {
        return String::new();
    }

    format!(
        "{}{}{}",
        "\x08".repeat(width),
        " ".repeat(width),
        "\x08".repeat(width)
    )
}

fn render_input_line(prompt: &str, input: &str, width: u16) -> String {
    let prompt_width = display_width(prompt);
    let total_width = width as usize;
    if total_width <= prompt_width {
        return truncate_to_width(prompt, width);
    }

    let available = total_width.saturating_sub(prompt_width);
    let fitted_input = fit_input_tail(input, available);
    format!("{prompt}{fitted_input}")
}

fn fit_input_tail(input: &str, width: usize) -> String {
    if display_width(input) <= width {
        return input.to_string();
    }

    let ellipsis = "…";
    let ellipsis_width = display_width(ellipsis);
    if width <= ellipsis_width {
        return truncate_to_width(ellipsis, width as u16);
    }

    let target_tail_width = width - ellipsis_width;
    let chars = input.chars().collect::<Vec<_>>();
    let mut collected = Vec::new();
    let mut used = 0usize;
    for ch in chars.iter().rev() {
        let ch_width = UnicodeWidthChar::width(*ch).unwrap_or(0);
        if used + ch_width > target_tail_width {
            break;
        }
        collected.push(*ch);
        used += ch_width;
    }
    collected.reverse();
    format!("{ellipsis}{}", collected.into_iter().collect::<String>())
}

fn truncate_to_width(text: &str, width: u16) -> String {
    let mut output = String::new();
    let mut used = 0usize;
    let max_width = width as usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > max_width {
            break;
        }
        output.push(ch);
        used += ch_width;
    }
    output
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

fn footer_spacer_lines(text: &str, footer_height: u16) -> u16 {
    let trailing_newlines = text.chars().rev().take_while(|ch| *ch == '\n').count() as u16;
    footer_height.saturating_sub(trailing_newlines.min(footer_height))
}

fn footer_growth_lines(old_footer_height: u16, new_footer_height: u16) -> u16 {
    new_footer_height.saturating_sub(old_footer_height)
}

fn render_running_input_echo(input: &PendingInput) -> String {
    let prompt = match input.delivery {
        PendingInputDelivery::InjectAtNextBoundary => "(guidance)> ",
        PendingInputDelivery::QueueAfterTurn => "(queued)> ",
    };
    format!("{prompt}{}", input.envelope.text.trim())
}

pub fn pending_input_from_running_line(line: &str) -> Option<PendingInput> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed == "/queue" {
        return None;
    }

    if let Some(queued) = trimmed.strip_prefix("/queue ") {
        let queued = queued.trim();
        if queued.is_empty() {
            return None;
        }
        return Some(PendingInput {
            envelope: InputEnvelope::new(InputSource::UserTerminal, "user.queued", queued),
            delivery: PendingInputDelivery::QueueAfterTurn,
        });
    }

    Some(PendingInput {
        envelope: InputEnvelope::new(InputSource::UserTerminal, "user.guidance", trimmed),
        delivery: PendingInputDelivery::InjectAtNextBoundary,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        backspace_sequence_for_char, display_width, fit_input_tail, footer_growth_lines,
        footer_spacer_lines, pending_input_from_running_line, render_input_line,
        render_running_input_echo, truncate_to_width, TerminalCommand, TerminalControllerState,
        TerminalMode,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use runtime::{InputSource, PendingInputDelivery};
    use std::sync::mpsc;

    #[test]
    fn running_line_becomes_user_guidance() {
        let input = pending_input_from_running_line("  先别改认证，只改 event router  ")
            .expect("guidance input");

        assert_eq!(input.envelope.source, InputSource::UserTerminal);
        assert_eq!(input.envelope.input_type, "user.guidance");
        assert_eq!(input.envelope.text, "先别改认证，只改 event router");
        assert_eq!(input.delivery, PendingInputDelivery::InjectAtNextBoundary);
    }

    #[test]
    fn queue_prefix_becomes_after_turn_input() {
        let input = pending_input_from_running_line("/queue 等当前任务结束后再写 smoke")
            .expect("queue input");

        assert_eq!(input.envelope.source, InputSource::UserTerminal);
        assert_eq!(input.envelope.input_type, "user.queued");
        assert_eq!(input.envelope.text, "等当前任务结束后再写 smoke");
        assert_eq!(input.delivery, PendingInputDelivery::QueueAfterTurn);
    }

    #[test]
    fn empty_running_lines_are_ignored() {
        assert!(pending_input_from_running_line("").is_none());
        assert!(pending_input_from_running_line("   ").is_none());
        assert!(pending_input_from_running_line("/queue   ").is_none());
    }

    #[test]
    fn terminal_command_types_cover_expected_modes() {
        let command = TerminalCommand::SetMode(TerminalMode::RunningGuidance);

        assert_eq!(
            command,
            TerminalCommand::SetMode(TerminalMode::RunningGuidance)
        );
    }

    #[test]
    fn setting_same_mode_is_noop_to_avoid_prompt_spam() {
        let (_permission_tx, permission_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let mut state = TerminalControllerState::new(Default::default(), permission_rx, event_tx);

        assert!(!state.set_mode(TerminalMode::IdlePrompt));
        assert!(state.set_mode(TerminalMode::RunningGuidance));
        assert!(!state.set_mode(TerminalMode::RunningGuidance));
        assert!(state.set_mode(TerminalMode::IdlePrompt));
    }

    #[test]
    fn key_release_events_do_not_echo_twice_on_windows() {
        let (_permission_tx, permission_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let mut state = TerminalControllerState::new(Default::default(), permission_rx, event_tx);

        state
            .handle_key(KeyEvent::new_with_kind(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ))
            .expect("release event should be ignored");

        assert!(state.line.is_empty());
    }

    #[test]
    fn backspace_sequence_clears_full_display_width() {
        assert_eq!(backspace_sequence_for_char('a'), "\x08 \x08");
        assert_eq!(backspace_sequence_for_char('中'), "\x08\x08  \x08\x08");
    }

    #[test]
    fn truncate_to_width_respects_wide_characters() {
        assert_eq!(truncate_to_width("中文abc", 4), "中文");
        assert_eq!(truncate_to_width("hello", 3), "hel");
    }

    #[test]
    fn fit_input_tail_keeps_tail_with_ellipsis() {
        assert_eq!(fit_input_tail("abcdefghijklmnopqrstuvwxyz", 6), "…vwxyz");
    }

    #[test]
    fn render_input_line_limits_total_display_width() {
        let rendered = render_input_line("(guidance)> ", "abcdefghijklmnopqrstuvwxyz", 20);
        assert!(display_width(&rendered) <= 20);
        assert!(rendered.starts_with("(guidance)> "));
    }

    #[test]
    fn footer_spacer_lines_depend_on_reserved_footer_height() {
        assert_eq!(footer_spacer_lines("chunk\n", 1), 0);
        assert_eq!(footer_spacer_lines("chunk\n", 2), 1);
        assert_eq!(footer_spacer_lines("chunk\n\n", 2), 0);
    }

    #[test]
    fn footer_growth_lines_only_counts_new_reserved_rows() {
        assert_eq!(footer_growth_lines(1, 2), 1);
        assert_eq!(footer_growth_lines(2, 7), 5);
        assert_eq!(footer_growth_lines(7, 2), 0);
    }

    #[test]
    fn render_running_input_echo_uses_delivery_specific_prompt() {
        let guidance = pending_input_from_running_line("please continue")
            .expect("guidance input should parse");
        let queued =
            pending_input_from_running_line("/queue run tests later").expect("queue input");

        assert_eq!(
            render_running_input_echo(&guidance),
            "(guidance)> please continue"
        );
        assert_eq!(
            render_running_input_echo(&queued),
            "(queued)> run tests later"
        );
    }
}
