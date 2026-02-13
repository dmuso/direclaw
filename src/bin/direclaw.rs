use direclaw::cli;

fn run() -> Result<(), String> {
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
