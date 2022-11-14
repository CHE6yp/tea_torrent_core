use clap::Parser;
use magnet_url::Magnet;

use std::sync::Arc;
use tea_torrent::Torrent;

fn main() {
    println!("{:?}", env!("CARGO_PKG_VERSION"));
    println!("\x1b]0;tTorrent\x07");
    let args = TTArgs::parse();
    let torrent = Torrent::new(args.torrent_file, args.destination, None);
    if let Some(link) = args.magnet_link {
        println!("{:?}", Magnet::new(&link));
    }
    let t = Arc::new(torrent);
    let jh = tea_torrent::run_torrent(t);
    let _result = jh.join();
}

/// CLI version of TeaTorrent. Downloads one torrent at a time.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct TTArgs {
    /// Torrent file path
    torrent_file: String,

    /// Download destination
    destination: Option<String>,

    //Magnet link
    #[clap(short, long)]
    magnet_link: Option<String>,
}
