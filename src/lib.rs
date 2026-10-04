pub mod agent_output;
pub mod agents;
pub mod conversations;
pub mod models;
pub mod runtime;
pub mod tools;

pub use runtime::Runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppResult {
    Help,
    Stream { prompt: String },
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
            let first = args.next().map(|arg| arg.as_ref().to_string());
            let stream = matches!(first.as_deref(), Some("--stream"));
            let prompt_parts = first
                .filter(|_| !stream)
                .into_iter()
                .chain(args.map(|arg| arg.as_ref().to_string()));
            let prompt = prompt_parts.collect::<Vec<_>>().join(" ");

            if prompt.trim().is_empty() {
                Err("ask needs a prompt".to_string())
            } else {
                Ok(AppResult::Stream { prompt })
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
            "  atlas ask [--stream] <prompt>",
            "  atlas help",
            "",
            "Responses stream by default. --stream remains available as an alias.",
        ]
        .join("\n"),
        AppResult::Stream { prompt } => prompt,
    }
}
