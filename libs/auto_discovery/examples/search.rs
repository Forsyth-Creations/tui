use auto_discovery::{Node};
use core::time::Duration;
use std::io;
use std::io::Write;

fn main() {
    let mut node = Node::new("base_station".to_string());
    println!("Searching...");
    let nodes = node.find_connections(Duration::from_secs(2));
    println!("Nodes: {:?}", nodes);

    // If the list is longer than 1, ask if you'd like to connect
    if nodes.len() > 0 {
        let first_node = nodes.get(0).unwrap();
        println!("\nEnter the pairing code for {:?}: ", first_node);
        io::stdout().flush().unwrap();

        let mut pairing_code = String::new();
        io::stdin().read_line(&mut pairing_code).unwrap();
        let pairing_code = pairing_code.trim();

        // Attempt connection
        match node.make_connection(first_node, pairing_code) {
            Ok(_) => println!("✓ Successfully connected to {}", first_node.name),
            Err(e) => println!("✗ Failed to connect: {}", e),
        }
    }
}