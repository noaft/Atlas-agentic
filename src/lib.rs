#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppResult {
    Help,
    Message(String),
}

pub fn handle_command<I, S>(args: I) -> Result<AppResult, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();

    match args.next().as_ref().map(AsRef::as_ref) {
        None | Some("help") | Some("--help") | Some("-h") => Ok(AppResult::Help),
        Some("ask") => {
            let prompt = args
                .map(|arg| arg.as_ref().to_string())
                .collect::<Vec<_>>()
                .join(" ");

            if prompt.trim().is_empty() {
                Err("ask needs a prompt".to_string())
            } else {
                Ok(AppResult::Message(agent::respond(&prompt)))
            }
        }
        Some(command) => Err(format!("unknown command: {command}")),
    }
}

pub fn render(result: AppResult) -> String {
    match result {
        AppResult::Help => [
            "Atlas Agentic",
            "",
            "Usage:",
            "  atlas ask <prompt>",
            "  atlas help",
        ]
        .join("\n"),
        AppResult::Message(message) => message,
    }
}

mod agent {
    pub fn respond(prompt: &str) -> String {
        format!("Atlas heard: {}", prompt.trim())
    }
}
