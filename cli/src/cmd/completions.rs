use clap::ValueEnum;

use crate::parser::CompletionsCommand;

/// Current shell completions supported by `clap_complete`
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    PowerShell,
}

pub fn handle_completions(command: CompletionsCommand) {
    match command {
        CompletionsCommand::Install { shell } => match install(shell) {
            Ok(_) => {}
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1)
            }
        },
        CompletionsCommand::Uninstall => match uninstall() {
            Ok(_) => {}
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1)
            }
        },
        CompletionsCommand::Show => show(),
    }
}

fn install(shell: Shell) -> std::io::Result<()> {
    Ok(())
}

fn uninstall() -> std::io::Result<()> {
    Ok(())
}

fn show() {}
