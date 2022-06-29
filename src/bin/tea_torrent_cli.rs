use clap::Parser;
use tea_torrent::run;

fn main() {
    println!("\x1b]0;tTorrent\x07");
    let args = Args::parse();
    run(args.torrent_file, args.destination);
}

/// CLI version of TeaTorrent. Downloads one torrent at a time.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Torrent file path
    torrent_file: String,

    /// Name of the person to greet
    destination: Option<String>,
}
