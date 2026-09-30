use atlas_agentic::{
    agents::{AgentRegistry, ChatAgent},
    handle_command,
    models::deepseek::{DeepSeekConfig, DeepSeekProvider},
    render, Runtime,
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

    let config = match DeepSeekConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(ChatAgent::new(
        Box::new(DeepSeekProvider::new(config)),
    ))]));

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
