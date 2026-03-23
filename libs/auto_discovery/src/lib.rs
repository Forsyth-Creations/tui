use std::{time::Duration};
use rand;
use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};
use std::net::{TcpListener, TcpStream};
use std::io::{BufRead, BufReader};
use hostname;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::env;

#[derive(Debug, Clone)]
pub struct DiscoveredNode {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub pairing_code: String,
}

#[derive(Debug, Clone)]
pub enum Status {
    Waiting,
    Connected,
    Failed
}

pub struct Node {
    pub name : String,
    pub status: Arc<Mutex<Status>>,
    pub discovered: Arc<Mutex<HashMap<String, DiscoveredNode>>>,
}

impl Node {

    pub fn new(name : String) -> Node {
        // if there is an environment variable in play, use that
        // instead of using the passed-in value
        let mut actual_name = name.clone();
        let key = "DESPEREAUX_NODE_NAME";
        match env::var(key) {
            Ok(val) => {
                println!("{key}: {val:?}");
                actual_name = val;
            },
            Err(_e) => (),
        }
        Self { name: actual_name, status: Arc::new(Mutex::new(Status::Waiting)), discovered: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn broadcast_existence(&self) {
        let pairing_code = rand::random_range(0..=9999);
        let code_str = format!("{:04}", pairing_code);

        let listener = TcpListener::bind("0.0.0.0:0").expect("Failed to bind");
        let port = listener.local_addr().unwrap().port();

        let mdns = ServiceDaemon::new().expect("Failed to create daemon");

        let monitor = mdns.monitor().expect("Failed to monitor daemon");
        std::thread::spawn(move || {
            while let Ok(event) = monitor.recv() {
                if let mdns_sd::DaemonEvent::Error(e) = event {
                    eprintln!("Daemon error: {e}");
                }
            }
        });

        let service_type = "_forsyth._tcp.local.";
        let hostname = format!("{}.local.", hostname::get().unwrap().to_string_lossy());
        let ip = self.get_local_ip();
        let instance_name = self.name.to_lowercase().replace(" ", "-");
        let name = self.name.clone();

        let properties = [("name", self.name.as_str())];

        let my_service = ServiceInfo::new(
            service_type,
            &instance_name,
            &hostname,
            ip.to_string().as_str(),
            port,
            &properties[..],
        ).unwrap();

        mdns.register(my_service).expect("Failed to register service");

        println!("-----------------------------------");
        println!("{} ready and waiting", name);
        println!("Pairing code: {}", code_str);
        println!("Listening on port {}", port);
        println!("Hostname: {}", hostname);
        println!("-----------------------------------");

        // Clone the Arc so the thread can update status
        let status = Arc::clone(&self.status);

        std::thread::spawn(move || {
            match listener.accept() {
                Ok((stream, addr)) => {
                    println!("Connection from {}", addr);
                    let success = handle_connection(stream, &code_str);
                    let mut s = status.lock().unwrap();
                    *s = if success { Status::Connected } else { Status::Failed };
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    *status.lock().unwrap() = Status::Failed;
                }
            }
            mdns.shutdown().unwrap();
        });

    }

    fn get_local_ip(&self) -> std::net::IpAddr {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.connect("8.8.8.8:80").unwrap();
        socket.local_addr().unwrap().ip()
    }


    pub fn find_connections(&self, timeout: Duration) -> Vec<DiscoveredNode> {
        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        let service_type = "_forsyth._tcp.local.";
        let receiver = mdns.browse(service_type).expect("Failed to browse");
        let discovered = Arc::new(Mutex::new(HashMap::new()));
        let my_name = self.name.to_lowercase().replace(" ", "-");

        let discovered_clone = Arc::clone(&discovered);

        let _handle = std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let name = info.get_fullname().to_string();

                        // Don't add ourselves
                        if name.starts_with(&my_name) {
                            println!("Cannot add myself");
                            continue;
                        }

                        let address = info.get_addresses()
                            .iter()
                            .next()
                            .map(|a| a.to_string())
                            .unwrap_or_default();

                        let port = info.get_port();

                        let pairing_code = info.get_properties()
                            .get("pairing_code")
                            .map(|p| p.val_str().to_string())
                            .unwrap_or_default();

                        let node = DiscoveredNode {
                            name: name.clone(),
                            address,
                            port,
                            pairing_code,
                        };

                        discovered_clone.lock().unwrap().insert(name, node);
                    }

                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        println!("Node disappeared: {}", fullname);
                        discovered_clone.lock().unwrap().remove(&fullname);
                    }

                    _ => {}
                }
            }
        });

        // Wait for the specified timeout
        std::thread::sleep(timeout);

        // The thread will continue running but we stop waiting
        // Extract discovered nodes
        let nodes: Vec<DiscoveredNode> = discovered
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();

        // Optional: shutdown the mdns daemon
        drop(mdns);

        nodes
    }


    pub fn make_connection(&self, node: &DiscoveredNode, pairing_code: &str) -> Result<(), anyhow::Error> {
        use std::io::Write;

        let addr = format!("{}:{}", node.address, node.port);
        println!("Connecting to {} at {}", node.name, addr);

        let stream = TcpStream::connect(&addr)?;
        let mut writer = stream.try_clone()?;
        let mut reader = BufReader::new(&stream);

        // Send the pairing code
        writeln!(writer, "{}", pairing_code)?;

        // Read response
        let mut response = String::new();
        reader.read_line(&mut response)?;

        println!("{:?}", &response);

        match response.trim() {
            "OK" => {
                println!("Pairing successful with {}!", node.name);
                Ok(())
            }
            "FAIL" => Err(anyhow::anyhow!("Wrong pairing code")),
            other  => Err(anyhow::anyhow!("Unexpected response: {}", other)),
        }
    }

}

fn handle_connection(stream: TcpStream, expected_code: &str) -> bool {
    use std::io::Write;
    let writer = stream.try_clone().expect("Failed to clone stream");
    let mut reader = BufReader::new(&stream);
    let mut writer = writer;

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let received_code = line.trim();

    if received_code == expected_code {
        println!("Pairing successful!");
        writeln!(writer, "OK").unwrap();
        true
    } else {
        println!("Wrong pairing code, got: {}", received_code);
        writeln!(writer, "FAIL").unwrap();
        false
    }
}