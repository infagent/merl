//! Command-line interface for Merl.

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match merl_cli::run(&arguments) {
        merl_cli::CliResponse::Success(output) => print!("{output}"),
        merl_cli::CliResponse::HumanError(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
        merl_cli::CliResponse::JsonError(error) => {
            print!("{}", error.as_json());
            std::process::exit(2);
        }
    }
}
