use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    handle_command,
    models::provider_from_definition,
    render,
    tools::{CreateFileTool, ListDirTool, PwdTool, ToolRegistry},
    Runtime,
};

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
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition, provider,
    ))]))
    .with_tools(ToolRegistry::new(vec![
        Box::new(PwdTool::new(".")),
        Box::new(ListDirTool::new(".")),
        Box::new(CreateFileTool::new(".")),
    ]));

    match handle_command(args, &runtime) {
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
