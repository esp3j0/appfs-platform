use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use runtime::{
    InputEnvelope, InputSource, PendingInput, PendingInputDelivery, SharedPendingInputQueue,
};
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
                if self.set_mode(mode) {
                    self.line.clear();
                    self.render_prompt()?;
                }
                Ok(false)
            }
            TerminalCommand::RenderPrompt => {
                self.render_prompt()?;
                Ok(false)
            }
            TerminalCommand::SetCompletions(completions) => {
                self.completions = completions;
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
        if self.mode != TerminalMode::PermissionPrompt {
            self.previous_mode = self.mode;
        }
        self.mode = TerminalMode::PermissionPrompt;
        self.permission_ticket = Some(ticket);
        self.line.clear();
        self.render_permission_prompt()
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
                writeln!(io::stdout())?;
                let submitted = std::mem::take(&mut self.line);
                match self.mode {
                    TerminalMode::IdlePrompt => {
                        let _ = self.event_tx.send(TerminalEvent::SubmittedLine(submitted));
                    }
                    TerminalMode::RunningGuidance => {
                        if let Some(input) = pending_input_from_running_line(&submitted) {
                            self.shared_queue.push(input);
                            writeln!(io::stdout(), "queued for the next model boundary")?;
                        }
                        self.render_prompt()?;
                    }
                    TerminalMode::PermissionPrompt => {}
                }
                Ok(false)
            }
            KeyCode::Backspace => {
                if self.line.pop().is_some() {
                    print!("\x08 \x08");
                    io::stdout().flush()?;
                }
                Ok(false)
            }
            KeyCode::Tab => {
                self.render_matching_completions()?;
                Ok(false)
            }
            KeyCode::Esc => {
                self.line.clear();
                writeln!(io::stdout())?;
                self.render_prompt()?;
                Ok(false)
            }
            KeyCode::Char(value) => {
                self.line.push(value);
                print!("{value}");
                io::stdout().flush()?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_permission_key(&mut self, key: KeyEvent) -> io::Result<bool> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                print!("y");
                io::stdout().flush()?;
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
                    print!("\x08 \x08");
                    io::stdout().flush()?;
                }
                Ok(false)
            }
            KeyCode::Char(value) => {
                self.line.push(value);
                print!("{value}");
                io::stdout().flush()?;
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
                    writeln!(io::stdout(), "^C")?;
                    self.render_prompt()?;
                    Ok(false)
                }
            }
            TerminalMode::RunningGuidance => {
                self.line.clear();
                writeln!(io::stdout(), "^C guidance cancelled")?;
                self.render_prompt()?;
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
        writeln!(io::stdout())?;
        if let Some(ticket) = self.permission_ticket.take() {
            let _ = ticket.response_tx.send(decision);
        }
        self.mode = self.previous_mode;
        self.line.clear();
        self.render_prompt()?;
        Ok(false)
    }

    fn render_prompt(&self) -> io::Result<()> {
        let prompt = match self.mode {
            TerminalMode::IdlePrompt => "> ",
            TerminalMode::RunningGuidance => "(guidance)> ",
            TerminalMode::PermissionPrompt => "",
        };
        if !prompt.is_empty() {
            print!("{prompt}");
            io::stdout().flush()?;
        }
        Ok(())
    }

    fn render_permission_prompt(&self) -> io::Result<()> {
        let Some(ticket) = self.permission_ticket.as_ref() else {
            return Ok(());
        };
        writeln!(io::stdout())?;
        writeln!(io::stdout(), "Permission approval required")?;
        writeln!(io::stdout(), "  Tool             {}", ticket.view.tool_name)?;
        writeln!(
            io::stdout(),
            "  Current mode     {}",
            ticket.view.current_mode
        )?;
        writeln!(
            io::stdout(),
            "  Required mode    {}",
            ticket.view.required_mode
        )?;
        if let Some(reason) = &ticket.view.reason {
            writeln!(io::stdout(), "  Reason           {reason}")?;
        }
        writeln!(io::stdout(), "  Input            {}", ticket.view.input)?;
        print!("Approve this tool call? [y/N]: ");
        io::stdout().flush()
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
        writeln!(io::stdout())?;
        writeln!(io::stdout(), "{}", matches.join("  "))?;
        self.render_prompt()?;
        print!("{}", self.line);
        io::stdout().flush()
    }
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
        pending_input_from_running_line, TerminalCommand, TerminalControllerState, TerminalMode,
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
}
