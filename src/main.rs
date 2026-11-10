use clap::Parser;
use void::start_app;

#[derive(Parser, Debug)]
#[command(name = "void", version, about = "Void Engine - Lightweight cross-platform web wrapper")]
struct Args {
    #[arg(long)]
    url: Option<String>,
}

fn main() -> wry::Result<()> {
    let args = Args::parse();
    start_app(args.url)
}
