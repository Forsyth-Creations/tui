use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
    Terminal,
};
use std::io;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use auto_discovery::{Node, DiscoveredNode};

enum ActivePane {
    Tasks,
    Output,
}

enum Page {
    Discovery,
    Searching,
    EnteringPasscode { input: String },
    Command,
    EnteringSshUser { input: String, address: String },
}

struct ConnectedNode {
    name: String,
    address: String,
    stream: std::net::TcpStream,
    remote_output: Arc<Mutex<String>>,
    tasks: Vec<String>,
    selected_task: usize,
    task_list_state: ListState,
}

impl ConnectedNode {
    fn new(name: String, address: String, stream: std::net::TcpStream, remote_output: Arc<Mutex<String>>, tasks: Vec<String>) -> Self {
        let mut task_list_state = ListState::default();
        if !tasks.is_empty() {
            task_list_state.select(Some(0));
        }
        Self { name, address, stream, remote_output, tasks, selected_task: 0, task_list_state }
    }

    fn next_task(&mut self) {
        if self.tasks.is_empty() { return; }
        self.selected_task = (self.selected_task + 1) % self.tasks.len();
        self.task_list_state.select(Some(self.selected_task));
    }

    fn prev_task(&mut self) {
        if self.tasks.is_empty() { return; }
        self.selected_task = if self.selected_task == 0 { self.tasks.len() - 1 } else { self.selected_task - 1 };
        self.task_list_state.select(Some(self.selected_task));
    }
}

struct App {
    node: Node,
    discovered_nodes: Vec<DiscoveredNode>,
    selected_host: usize,    // index into filtered_indices()
    selected_connected: usize,
    active_pane: ActivePane,
    output: String,
    should_quit: bool,
    page: Page,
    connected_nodes: Vec<ConnectedNode>,
    search_query: String,
    pending_ssh: Option<(String, String)>, // (username, address)
}

impl App {
    pub fn new(node: Node, initial_nodes: Vec<DiscoveredNode>) -> Self {
        Self {
            node,
            discovered_nodes: initial_nodes,
            selected_host: 0,
            selected_connected: 0,
            active_pane: ActivePane::Tasks,
            output: String::new(),
            should_quit: false,
            page: Page::Discovery,
            connected_nodes: Vec::new(),
            search_query: String::new(),
            pending_ssh: None,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        if self.search_query.is_empty() {
            (0..self.discovered_nodes.len()).collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.discovered_nodes.iter().enumerate()
                .filter(|(_, n)| n.name.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect()
        }
    }

    fn rescan(&mut self) {
        self.discovered_nodes = self.node.find_connections(Duration::from_secs(3));
        let flen = self.filtered_indices().len();
        if self.selected_host >= flen && flen > 0 {
            self.selected_host = flen - 1;
        }
    }

    fn next_host(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 { return; }
        self.selected_host = (self.selected_host + 1) % len;
    }

    fn prev_host(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 { return; }
        if self.selected_host == 0 { self.selected_host = len - 1; } else { self.selected_host -= 1; }
    }

    fn next_connected(&mut self) {
        let len = self.connected_nodes.len();
        if len == 0 { return; }
        self.selected_connected = (self.selected_connected + 1) % len;
    }

    fn prev_connected(&mut self) {
        let len = self.connected_nodes.len();
        if len == 0 { return; }
        if self.selected_connected == 0 { self.selected_connected = len - 1; } else { self.selected_connected -= 1; }
    }
}

fn ui_discovery(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let filtered = app.filtered_indices();

    let tab_titles: Vec<Line> = if filtered.is_empty() {
        let msg = if app.discovered_nodes.is_empty() { " No nodes discovered " } else { " No matches " };
        vec![Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray)))]
    } else {
        filtered.iter().map(|&i| {
            let h = &app.discovered_nodes[i];
            let already = app.connected_nodes.iter().any(|c| c.name == h.name);
            let label = if already { format!("✓ {}", h.name) } else { h.name.clone() };
            Line::from(Span::raw(label))
        }).collect()
    };

    let tabs = Tabs::new(tab_titles)
        .select(app.selected_host)
        .block(Block::default().title("Discovered Nodes").borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
        .divider("|");
    frame.render_widget(tabs, outer[0]);

    let is_searching = matches!(app.page, Page::Searching);
    let search_border = if is_searching {
        Style::default().fg(Color::Yellow)
    } else if !app.search_query.is_empty() {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let search_text = if is_searching {
        format!("/{}█", app.search_query)
    } else if !app.search_query.is_empty() {
        format!("/{}", app.search_query)
    } else {
        String::new()
    };
    let search_bar = Paragraph::new(search_text)
        .block(Block::default().title("Search").borders(Borders::ALL).border_style(search_border));
    frame.render_widget(search_bar, outer[1]);

    let detail_text = if filtered.is_empty() {
        if app.discovered_nodes.is_empty() {
            "No nodes discovered yet.\n\nPress 'r' to rescan.".to_string()
        } else {
            format!("No nodes match \"{}\".", app.search_query)
        }
    } else if let Some(&real_idx) = filtered.get(app.selected_host) {
        let node = &app.discovered_nodes[real_idx];
        let already = app.connected_nodes.iter().any(|c| c.name == node.name);
        let base = format!(
            "Node:    {}\nAddress: {}\nPort:    {}{}\n",
            node.name, node.address, node.port,
            if already { "  (already connected)" } else { "" }
        );
        match &app.page {
            Page::EnteringPasscode { input } => {
                format!(
                    "{}\nEnter pairing code (shown on the target device):\n  > {}█\n\n[Enter] confirm   [Esc] cancel",
                    base, input
                )
            }
            _ => format!("{}\n{}", base, app.output),
        }
    } else {
        app.output.clone()
    };

    let detail = Paragraph::new(detail_text)
        .block(Block::default().title("Node Details").borders(Borders::ALL));
    frame.render_widget(detail, outer[2]);

    let status = match &app.page {
        Page::Searching => {
            " [Enter] confirm  [Esc] clear & exit search  │  type to filter nodes by name".to_string()
        }
        Page::EnteringPasscode { .. } => {
            " [Enter] confirm  [Esc] cancel  │  type the pairing code shown on the target device".to_string()
        }
        _ => {
            let n = app.connected_nodes.len();
            let selected_is_connected = filtered.get(app.selected_host)
                .map(|&i| app.connected_nodes.iter().any(|c| c.name == app.discovered_nodes[i].name))
                .unwrap_or(false);
            format!(
                " [←/→] switch node  [Enter] pair  [/] search  [r] rescan  [q] quit{}{}  │  discovered: {}  connected: {}",
                if n > 0 { "  [c] command page" } else { "" },
                if selected_is_connected { "  [x] disconnect" } else { "" },
                app.discovered_nodes.len(),
                n,
            )
        }
    };
    let status_bar = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status_bar, outer[3]);
}

fn ui_command(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let tab_titles: Vec<Line> = if app.connected_nodes.is_empty() {
        vec![Line::from(Span::styled(" No connected nodes ", Style::default().fg(Color::DarkGray)))]
    } else {
        app.connected_nodes.iter()
            .map(|n| Line::from(Span::raw(n.name.clone())))
            .collect()
    };

    let tabs = Tabs::new(tab_titles)
        .select(app.selected_connected)
        .block(Block::default().title("Connected Nodes").borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        .divider("|");
    frame.render_widget(tabs, outer[0]);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(outer[1]);

    let task_items: Vec<ListItem> = app.connected_nodes.get(app.selected_connected)
        .map(|cn| cn.tasks.iter().map(|t| ListItem::new(t.clone())).collect())
        .unwrap_or_default();

    let task_border_style = match app.active_pane {
        ActivePane::Tasks  => Style::default().fg(Color::Green),
        ActivePane::Output => Style::default().fg(Color::DarkGray),
    };

    let task_list = List::new(task_items)
        .block(Block::default().title("Tasks").borders(Borders::ALL).border_style(task_border_style))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Green))
        .highlight_symbol("▶ ");

    if let Some(cn) = app.connected_nodes.get_mut(app.selected_connected) {
        frame.render_stateful_widget(task_list, content[0], &mut cn.task_list_state);
    } else {
        frame.render_widget(task_list, content[0]);
    }

    let output_border_style = match app.active_pane {
        ActivePane::Output => Style::default().fg(Color::Green),
        ActivePane::Tasks  => Style::default().fg(Color::DarkGray),
    };

    let output_text = match &app.page {
        Page::EnteringSshUser { input, address } => {
            format!("SSH into {}\n\nUsername: {}█\n\n[Enter] connect   [Esc] cancel", address, input)
        }
        _ => {
            if let Some(cn) = app.connected_nodes.get(app.selected_connected) {
                let remote = cn.remote_output.lock().unwrap().clone();
                if app.output.is_empty() { remote } else { format!("{}\n{}", app.output, remote) }
            } else {
                "No nodes connected.\n\nPress 'd' to go to the discovery page.".to_string()
            }
        }
    };

    let output = Paragraph::new(output_text)
        .block(Block::default().title("Output").borders(Borders::ALL).border_style(output_border_style));
    frame.render_widget(output, content[1]);

    let node_label = app.connected_nodes.get(app.selected_connected)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "none".to_string());

    let status = match &app.page {
        Page::EnteringSshUser { .. } => {
            " [Enter] open SSH session  [Esc] cancel  │  type the username to connect as".to_string()
        }
        _ => format!(
            " [←/→] switch node  [↑/↓] select task  [Enter] send task  [s] ssh  [Tab] focus  [d] discovery  [q] quit  │  node: {}  connected: {}",
            node_label, app.connected_nodes.len()
        ),
    };
    let status_bar = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status_bar, outer[2]);
}

fn ui(frame: &mut ratatui::Frame, app: &mut App) {
    match app.page {
        Page::Command | Page::EnteringSshUser { .. } => ui_command(frame, app),
        _ => ui_discovery(frame, app),
    }
}

fn main() -> io::Result<()> {
    let node = Node::new("forsyth-tui".to_string());

    println!("Running initial scan for 3 seconds...");
    let initial_nodes = node.find_connections(Duration::from_secs(3));
    println!("Found {} nodes", initial_nodes.len());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(node, initial_nodes);

    loop {
        terminal.draw(|frame| ui(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match &mut app.page {
                    Page::Searching => {
                        match key.code {
                            KeyCode::Esc => {
                                app.search_query.clear();
                                app.selected_host = 0;
                                app.page = Page::Discovery;
                            }
                            KeyCode::Enter => {
                                app.selected_host = 0;
                                app.page = Page::Discovery;
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.selected_host = 0;
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.selected_host = 0;
                            }
                            _ => {}
                        }
                    }
                    Page::EnteringPasscode { input } => {
                        match key.code {
                            KeyCode::Esc => {
                                app.page = Page::Discovery;
                                app.output = "Connection cancelled.".to_string();
                            }
                            KeyCode::Backspace => { input.pop(); }
                            KeyCode::Char(c) => { input.push(c); }
                            KeyCode::Enter => {
                                let passcode = input.clone();
                                let real_idx = app.filtered_indices().get(app.selected_host).copied();
                                let node_clone = real_idx.and_then(|i| app.discovered_nodes.get(i)).cloned();
                                app.page = Page::Discovery;
                                if let Some(target) = node_clone {
                                    if app.connected_nodes.iter().any(|c| c.name == target.name) {
                                        app.output = format!("{} is already connected.", target.name);
                                    } else {
                                        match app.node.make_connection(&target, &passcode) {
                                            Ok((stream, tasks)) => {
                                                let reader_stream = stream.try_clone().unwrap();
                                                let remote_output = Arc::new(Mutex::new(String::new()));
                                                let output_ref = Arc::clone(&remote_output);
                                                std::thread::spawn(move || {
                                                    use std::io::BufRead;
                                                    let mut reader = std::io::BufReader::new(reader_stream);
                                                    loop {
                                                        let mut line = String::new();
                                                        match reader.read_line(&mut line) {
                                                            Ok(0) | Err(_) => break,
                                                            Ok(_) => { output_ref.lock().unwrap().push_str(&line); }
                                                        }
                                                    }
                                                });
                                                let cn = ConnectedNode::new(
                                                    target.name.clone(),
                                                    target.address.clone(),
                                                    stream,
                                                    remote_output,
                                                    tasks,
                                                );
                                                app.connected_nodes.push(cn);
                                                app.selected_connected = app.connected_nodes.len() - 1;
                                                app.output = String::new();
                                                app.page = Page::Command;
                                            }
                                            Err(e) => {
                                                app.output = format!("✗ Failed to connect to {}: {}", target.name, e);
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Page::EnteringSshUser { input, address } => {
                        match key.code {
                            KeyCode::Esc => { app.page = Page::Command; }
                            KeyCode::Backspace => { input.pop(); }
                            KeyCode::Char(c) => { input.push(c); }
                            KeyCode::Enter => {
                                let username = input.clone();
                                let addr = address.clone();
                                app.page = Page::Command;
                                if !username.is_empty() {
                                    app.pending_ssh = Some((username, addr));
                                }
                            }
                            _ => {}
                        }
                    }
                    Page::Discovery => {
                        match key.code {
                            KeyCode::Char('q') => app.should_quit = true,
                            KeyCode::Char('/') => { app.page = Page::Searching; }
                            KeyCode::Char('r') => {
                                app.output = "Rescanning...".to_string();
                                app.rescan();
                                app.output = format!("Found {} nodes.", app.discovered_nodes.len());
                            }
                            KeyCode::Char('c') => {
                                if !app.connected_nodes.is_empty() {
                                    app.page = Page::Command;
                                }
                            }
                            KeyCode::Right => app.next_host(),
                            KeyCode::Left  => app.prev_host(),
                            KeyCode::Enter => {
                                let real_idx = app.filtered_indices().get(app.selected_host).copied();
                                if let Some(i) = real_idx {
                                    let name = app.discovered_nodes[i].name.clone();
                                    if let Some(pos) = app.connected_nodes.iter().position(|c| c.name == name) {
                                        app.selected_connected = pos;
                                        app.page = Page::Command;
                                    } else {
                                        app.page = Page::EnteringPasscode { input: String::new() };
                                        app.output = String::new();
                                    }
                                } else {
                                    app.output = "No node selected. Press 'r' to rescan.".to_string();
                                }
                            }
                            KeyCode::Char('x') => {
                                let real_idx = app.filtered_indices().get(app.selected_host).copied();
                                if let Some(i) = real_idx {
                                    let name = app.discovered_nodes[i].name.clone();
                                    if let Some(idx) = app.connected_nodes.iter().position(|c| c.name == name) {
                                        app.connected_nodes.remove(idx);
                                        if app.selected_connected >= app.connected_nodes.len() && !app.connected_nodes.is_empty() {
                                            app.selected_connected = app.connected_nodes.len() - 1;
                                        }
                                        app.output = format!("Disconnected from {}.", name);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Page::Command => {
                        match key.code {
                            KeyCode::Char('q') => app.should_quit = true,
                            KeyCode::Char('d') => { app.page = Page::Discovery; }
                            KeyCode::Char('s') => {
                                if let Some(cn) = app.connected_nodes.get(app.selected_connected) {
                                    let address = cn.address.clone();
                                    app.page = Page::EnteringSshUser { input: String::new(), address };
                                }
                            }
                            KeyCode::Right => app.next_connected(),
                            KeyCode::Left  => app.prev_connected(),
                            KeyCode::Down  => {
                                let idx = app.selected_connected;
                                if let Some(cn) = app.connected_nodes.get_mut(idx) {
                                    cn.next_task();
                                }
                            }
                            KeyCode::Up => {
                                let idx = app.selected_connected;
                                if let Some(cn) = app.connected_nodes.get_mut(idx) {
                                    cn.prev_task();
                                }
                            }
                            KeyCode::Tab => {
                                app.active_pane = match app.active_pane {
                                    ActivePane::Tasks  => ActivePane::Output,
                                    ActivePane::Output => ActivePane::Tasks,
                                };
                            }
                            KeyCode::Enter => {
                                let idx = app.selected_connected;
                                if let Some(cn) = app.connected_nodes.get_mut(idx) {
                                    if let Some(task) = cn.tasks.get(cn.selected_task).cloned() {
                                        use std::io::Write;
                                        if writeln!(cn.stream, "{}", task).is_ok() {
                                            app.output = format!("→ {}", task);
                                        } else {
                                            let name = cn.name.clone();
                                            app.connected_nodes.remove(idx);
                                            if app.selected_connected >= app.connected_nodes.len() && !app.connected_nodes.is_empty() {
                                                app.selected_connected = app.connected_nodes.len() - 1;
                                            }
                                            app.output = format!("Connection to {} lost.", name);
                                            if app.connected_nodes.is_empty() {
                                                app.page = Page::Discovery;
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if let Some((user, addr)) = app.pending_ssh.take() {
            disable_raw_mode()?;
            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

            std::process::Command::new("ssh")
                .arg(format!("{}@{}", user, addr))
                .status()
                .ok();

            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            terminal.clear()?;
        }

        if app.should_quit { break; }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
