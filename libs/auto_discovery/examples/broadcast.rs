use auto_discovery::{Node, Status};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let param = &args[1];

    let mut node = Node::new(param.to_string());

    // Optional: port range as "start-end", e.g. "8000-8100"
    if let Some(range) = args.get(2).and_then(|s| {
        let (a, b) = s.split_once('-')?;
        Some(a.parse::<u16>().ok()?..=b.parse::<u16>().ok()?)
    }) {
        node.port_range(range);
    }

    // Optional: hard-coded pairing code, e.g. "1234"
    if let Some(code) = args.get(3) {
        node.passcode(code);
    }

    node.broadcast_existence();

    let mut wait_count = 0u32;
    loop {
        let status = node.status.lock().unwrap().clone();
        match status {
            Status::Waiting => {
                wait_count += 1;
                print!("\rWaiting for connection... ({})", wait_count);
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Status::Connected => {
                if wait_count > 0 {
                    println!();
                    wait_count = 0;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Status::Failed => {
                if wait_count > 0 { println!(); }
                eprintln!("Connection failed or closed.");
                break;
            }
        }
    }
}