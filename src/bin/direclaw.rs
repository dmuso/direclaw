use direclaw::cli;

fn output_header() -> &'static str {
    "DireClaw\nDireClaw is a file-backed multi-agent orchestration runtime for channel-driven workflows.\nGitHub: https://github.com/dmuso/direclaw"
}

fn print_header() {
    println!("{}\n", output_header());
}

fn run() -> Result<(), String> {
    print_header();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = cli::run(args)?;
    println!("{output}");
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
