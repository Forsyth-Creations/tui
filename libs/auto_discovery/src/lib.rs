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
    pub name: String,
    pub status: Arc<Mutex<Status>>,
    pub discovered: Arc<Mutex<HashMap<String, DiscoveredNode>>>,
    pending_port_range: Option<std::ops::RangeInclusive<u16>>,
    pending_passcode: Option<String>,
    tasks: Vec<String>,
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
        Self {
            name: actual_name,
            status: Arc::new(Mutex::new(Status::Waiting)),
            discovered: Arc::new(Mutex::new(HashMap::new())),
            pending_port_range: None,
            pending_passcode: None,
            tasks: Vec::new(),
        }
    }

    pub fn tasks(&mut self, tasks: Vec<String>) -> &mut Self {
        self.tasks = tasks;
        self
    }

    pub fn port_range(&mut self, range: std::ops::RangeInclusive<u16>) -> &mut Self {
        self.pending_port_range = Some(range);
        self
    }

    pub fn passcode(&mut self, code: impl Into<String>) -> &mut Self {
        self.pending_passcode = Some(code.into());
        self
    }

    pub fn broadcast_existence(&mut self) {
        let code_str = match self.pending_passcode.take() {
            Some(code) => code,
            None => format!("{:04}", rand::random_range(0..=9999)),
        };

        let listener = match self.pending_port_range.take() {
            None => TcpListener::bind("0.0.0.0:0").expect("Failed to bind"),
            Some(range) => {
                let mut bound = None;
                for port in range {
                    if let Ok(l) = TcpListener::bind(format!("0.0.0.0:{}", port)) {
                        bound = Some(l);
                        break;
                    }
                }
                bound.expect("No ports available in the specified range")
            }
        };
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
        let tasks = self.tasks.clone();

        std::thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        println!("Connection from {}", addr);
                        match handle_connection(stream, &code_str, &tasks) {
                            Some(connected) => {
                                *status.lock().unwrap() = Status::Connected;
                                let mut reader = BufReader::new(connected);
                                loop {
                                    let mut cmd = String::new();
                                    match reader.read_line(&mut cmd) {
                                        Ok(0) => {
                                            println!("Connection closed by peer. Re-broadcasting...");
                                            break;
                                        }
                                        Ok(_) => {
                                            println!("Command: {}", cmd.trim());
                                        }
                                        Err(e) => {
                                            eprintln!("Read error: {}. Re-broadcasting...", e);
                                            break;
                                        }
                                    }
                                }
                                *status.lock().unwrap() = Status::Waiting;
                            }
                            None => {
                                // Wrong pairing code — keep listening, status stays Waiting
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to accept connection: {}", e);
                        *status.lock().unwrap() = Status::Failed;
                        break;
                    }
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


    pub fn make_connection(&self, node: &DiscoveredNode, pairing_code: &str) -> Result<(TcpStream, Vec<String>), anyhow::Error> {
        use std::io::Write;

        let addr = format!("{}:{}", node.address, node.port);

        let stream = TcpStream::connect(&addr)?;
        let mut writer = stream.try_clone()?;

        // Send the pairing code
        writeln!(writer, "{}", pairing_code)?;

        // Read response then task list
        let (response, tasks) = {
            let mut reader = BufReader::new(&stream);

            let mut line = String::new();
            reader.read_line(&mut line)?;
            let response = line.trim().to_string();

            let tasks = if response == "OK" {
                let mut count_line = String::new();
                reader.read_line(&mut count_line)?;
                let count: usize = count_line.trim()
                    .strip_prefix("TASKS ")
                    .ok_or_else(|| anyhow::anyhow!("Expected TASKS line, got: {}", count_line.trim()))?
                    .parse()?;
                let mut tasks = Vec::with_capacity(count);
                for _ in 0..count {
                    let mut task = String::new();
                    reader.read_line(&mut task)?;
                    tasks.push(task.trim().to_string());
                }
                tasks
            } else {
                Vec::new()
            };

            (response, tasks)
        };

        match response.as_str() {
            "OK"   => Ok((stream, tasks)),
            "FAIL" => Err(anyhow::anyhow!("Wrong pairing code")),
            other  => Err(anyhow::anyhow!("Unexpected response: {}", other)),
        }
    }

}

fn handle_connection(stream: TcpStream, expected_code: &str, tasks: &[String]) -> Option<TcpStream> {
    use std::io::Write;
    let mut writer = stream.try_clone().expect("Failed to clone stream");

    let received = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line.trim().to_string()
    };

    if received == expected_code {
        writeln!(writer, "OK").unwrap();
        writeln!(writer, "TASKS {}", tasks.len()).unwrap();
        for task in tasks {
            writeln!(writer, "{}", task).unwrap();
        }
        Some(stream)
    } else {
        writeln!(writer, "FAIL").unwrap();
        None
    }
}