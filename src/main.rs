use std::io::Write;

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    handle_command,
    models::provider_from_definition,
    render,
    tools::{CreateFileTool, ListDirTool, PwdTool, ToolRegistry},
    AppResult, Runtime,
};

#[cfg(unix)]
use atlas_agentic::tools::ExecCommandTool;

fn main() {
    load_dotenv();

    let args = std::env::args().skip(1).collect::<Vec<_>>();

    if matches!(
        args.first().map(String::as_str),
        None | Some("help") | Some("--help") | Some("-h")
    ) {
        println!("{}", render(atlas_agentic::AppResult::Help));
        return;
    }

    let definition = match LocalAgentDefinition::from_file("agents/atlas.toml") {
        Ok(definition) => definition,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    };
    let provider = match provider_from_definition(&definition) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    };
    let tools = ToolRegistry::new(vec![
        Box::new(PwdTool::new(".")),
        Box::new(ListDirTool::new(".")),
        Box::new(CreateFileTool::new(".")),
    ]);
    #[cfg(unix)]
    let tools = {
        let mut tools = tools;
        tools.register(Box::new(ExecCommandTool::new(".")));
        tools
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition, provider,
    ))]))
    .with_tools(tools);

    match handle_command(args) {
        Ok(AppResult::Stream { prompt }) => {
            let mut stdout = std::io::stdout();
            let result = runtime.ask_stream(&prompt, &mut |delta| {
                print!("{delta}");
                let _ = stdout.flush();
            });

            match result {
                Ok(_) => println!(),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
            }
        }
        Ok(result) => println!("{}", render(result)),
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    }
}

fn load_dotenv() {
    if std::env::var_os("ATLAS_SKIP_DOTENV").is_none() {
        let _ = dotenvy::dotenv();
    }
}
