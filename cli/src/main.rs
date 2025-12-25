use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;

use agentfs::{
    cmd::{self, completions::handle_completions},
    get_runtime,
    parser::{Args, Command, FsCommand},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    reset_sigpipe();

    CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();

    let default_env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::ERROR.into())
        .from_env_lossy();
    if let Err(e) = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_line_number(true)
                .with_thread_ids(true),
        )
        .with(default_env_filter.add_directive("rustyline=off".parse().unwrap()))
        .try_init()
    {
        println!("Unable to setup tracing appender: {e:?}");
    }

    match args.command {
        Command::Init {
            id,
            sync_config_path,
            force,
            base,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::init::init_database(id, sync_config_path, force, base))
            {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Run {
            allow,
            no_default_allows,
            experimental_sandbox,
            strace,
            command,
            args,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::handle_run_command(
                allow,
                no_default_allows,
                experimental_sandbox,
                strace,
                command,
                args,
            )) {
                eprintln!("Error: {e:?}");
                std::process::exit(1);
            }
        }
        Command::Mount {
            id_or_path,
            sync_config_path,
            mountpoint,
            auto_unmount,
            allow_root,
            foreground,
            uid,
            gid,
        } => {
            if let Err(e) = cmd::mount(cmd::MountArgs {
                id_or_path,
                sync_config_path,
                mountpoint,
                auto_unmount,
                allow_root,
                foreground,
                uid,
                gid,
            }) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Diff {
            id_or_path,
            sync_config_path,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::fs::diff_filesystem(id_or_path)) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Fs {
            command,
            id_or_path,
            sync_config_path,
        } => {
            let rt = get_runtime();
            match command {
                FsCommand::Ls { fs_path } => {
                    if let Err(e) = rt.block_on(cmd::fs::ls_filesystem(
                        &mut std::io::stdout(),
                        id_or_path,
                        &fs_path,
                    )) {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                FsCommand::Cat { file_path } => {
                    if let Err(e) = rt.block_on(cmd::fs::cat_filesystem(
                        &mut std::io::stdout(),
                        id_or_path,
                        &file_path,
                    )) {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Command::Completions { command } => handle_completions(command),
        #[cfg(unix)]
        Command::Nfs {
            id_or_path,
            sync_config_path,
            bind,
            port,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::nfs::handle_nfs_command(id_or_path, bind, port)) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Reset SIGPIPE to the default behavior (terminate the process) so that
/// piping output to tools like `head` doesn't cause a panic.
#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}
