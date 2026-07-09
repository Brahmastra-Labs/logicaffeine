//! `largo completions` — shell completion script generation.

use clap::CommandFactory;
use clap_complete::Shell;

/// Handle `largo completions <shell>`: write the completion script to stdout.
pub(crate) fn cmd_completions(shell: Shell) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(shell, &mut cmd, "largo", &mut std::io::stdout());
    Ok(())
}
