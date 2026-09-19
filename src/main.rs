//! Command-line interface for Merl.

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match merl_cli::run(&arguments) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            if arguments.iter().any(|argument| argument == "--json")
                || arguments
                    .windows(2)
                    .any(|pair| pair[0] == "--format" && pair[1] == "json")
            {
                print!("{}", error.as_json());
            } else {
                eprintln!("{error}");
            }
            std::process::exit(2);
        }
    }
}
