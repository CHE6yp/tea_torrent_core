use t_torrent::run;

fn main() {
	println!("\x1b]0;tTorrent\x07");
    let args: Vec<String> = std::env::args().collect();
	run(args);
}