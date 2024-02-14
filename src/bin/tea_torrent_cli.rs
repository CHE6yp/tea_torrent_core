use clap::{Args, Parser};
use magnet_url::Magnet;

use std::sync::Arc;
use tea_torrent::Torrent;

fn main() {
    println!("tTorrent {}\n", env!("CARGO_PKG_VERSION"));
    // println!("\x1b]0;tTorrent\x07");
    let args = TTArgs::parse();

    if let Some(link) = args.source.magnet_link {
        println!("Magnet links are not yet supported");
        println!("{:?}", Magnet::new(&link));
        return
    }
    let torrent = Torrent::new(args.source.torrent_file.unwrap(), args.destination, None);

    let t = Arc::new(torrent);
    let jh = tea_torrent::run_torrent(t);
    let _result = jh.join();
}

/// CLI version of TeaTorrent. Downloads one torrent at a time.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct TTArgs {
    /// Torrent file or magnet link
    #[clap(flatten)]
    source: SourceArgument,

    /// Download destination
    destination: Option<String>,
}

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
pub struct SourceArgument {
    /// Torrent file path
    #[clap(short = 'f', long)]
    torrent_file: Option<String>,
    /// Magnet link
    #[clap(short = 'm', long)]
    magnet_link: Option<String>,
}


