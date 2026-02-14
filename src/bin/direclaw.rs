use direclaw::cli;

fn output_header() -> &'static str {
    "DireClaw\nDireClaw is a file-backed multi-agent orchestration runtime for channel-driven workflows.\nGitHub: https://github.com/dmuso/direclaw"
}

fn print_with_header(message: &str) {
    println!("{}\n\n{message}", output_header());
}

fn eprint_with_header(message: &str) {
    eprintln!("{}\n\n{message}", output_header());
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = cli::run(args)?;
    print_with_header(&output);
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprint_with_header(&err);
        std::process::exit(1);
    }
}
