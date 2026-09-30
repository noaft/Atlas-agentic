pub mod agent_output;
pub mod agents;
pub mod models;
pub mod runtime;
pub mod tools;

pub use runtime::Runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppResult {
    Help,
    Message(String),
}

pub fn handle_command<I, S>(args: I, runtime: &Runtime) -> Result<AppResult, String>
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
                runtime.ask(&prompt).map(AppResult::Message)
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
