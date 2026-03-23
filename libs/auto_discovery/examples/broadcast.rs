use auto_discovery::{Node, Status};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let param = &args[1];

    let mut node = Node::new(param.to_string());
    node.broadcast_existence(); // returns immediately now

    loop {
        let status = node.status.lock().unwrap().clone();
        match status {
            Status::Waiting => {
                println!("Waiting for connection...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Status::Connected => {
                println!("Connected! Ready to run tasks.");
                break;
            }
            Status::Failed => {
                eprintln!("Connection failed.");
                break;
            }
        }
    }
}