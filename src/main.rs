use atlas_agentic::{handle_command, render};

fn main() {
    match handle_command(std::env::args().skip(1)) {
        Ok(result) => println!("{}", render(result)),
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    }
}
