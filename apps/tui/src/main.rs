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
use auto_discovery::{Node, DiscoveredNode};

enum ActivePane {
    Tasks,
    Output,
}

struct App {
    node: Node,
    discovered_nodes: Vec<DiscoveredNode>,  // Changed: store discovered nodes here
    tasks: Vec<String>,
    selected_host: usize,
    selected_task: usize,
    task_list_state: ListState,
    active_pane: ActivePane,
    output: String,
    should_quit: bool,
    last_scan: std::time::Instant,  // Track when we last scanned
}

impl App {
    pub fn new(node: Node, tasks: Vec<String>, initial_nodes: Vec<DiscoveredNode>) -> Self {
        let mut task_list_state = ListState::default();
        task_list_state.select(Some(0));
        Self {
            node,
            discovered_nodes: initial_nodes,
            tasks,
            selected_host: 0,
            selected_task: 0,
            task_list_state,
            active_pane: ActivePane::Tasks,
            output: String::new(),
            should_quit: false,
            last_scan: std::time::Instant::now(),
        }
    }

    // Rescan for nodes
    fn rescan(&mut self) {
        self.discovered_nodes = self.node.find_connections(Duration::from_secs(3));
        self.last_scan = std::time::Instant::now();

        // Adjust selected_host if it's now out of bounds
        if self.selected_host >= self.discovered_nodes.len() && !self.discovered_nodes.is_empty() {
            self.selected_host = self.discovered_nodes.len() - 1;
        }
    }

    fn next_host(&mut self) {
        let len = self.discovered_nodes.len();
        if len == 0 { return; }
        self.selected_host = (self.selected_host + 1) % len;
    }

    fn prev_host(&mut self) {
        let len = self.discovered_nodes.len();
        if len == 0 { return; }
        if self.selected_host == 0 {
            self.selected_host = len - 1;
        } else {
            self.selected_host -= 1;
        }
    }

    fn next_task(&mut self) {
        let next = (self.selected_task + 1) % self.tasks.len();
        self.selected_task = next;
        self.task_list_state.select(Some(next));
    }

    fn prev_task(&mut self) {
        let prev = if self.selected_task == 0 {
            self.tasks.len() - 1
        } else {
            self.selected_task - 1
        };
        self.selected_task = prev;
        self.task_list_state.select(Some(prev));
    }
}

fn ui(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // --- Tabs (discovered hosts) ---
    let tab_titles: Vec<Line> = if app.discovered_nodes.is_empty() {
        vec![Line::from(Span::styled(
            " No nodes discovered ",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.discovered_nodes.iter()
            .map(|h| Line::from(Span::raw(h.name.clone())))
            .collect()
    };

    let tabs = Tabs::new(tab_titles)
        .select(app.selected_host)
        .block(Block::default().title("Discovered Nodes").borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
        .divider("|");

    frame.render_widget(tabs, outer[0]);

    // --- Main content ---
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(outer[1]);

    let task_items: Vec<ListItem> = app.tasks.iter()
        .map(|t| ListItem::new(t.clone()))
        .collect();

    let task_border_style = match app.active_pane {
        ActivePane::Tasks  => Style::default().fg(Color::Blue),
        ActivePane::Output => Style::default().fg(Color::DarkGray),
    };

    let task_list = List::new(task_items)
        .block(Block::default().title("Tasks").borders(Borders::ALL).border_style(task_border_style))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Blue))
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(task_list, content[0], &mut app.task_list_state);

    let output_border_style = match app.active_pane {
        ActivePane::Output => Style::default().fg(Color::Blue),
        ActivePane::Tasks  => Style::default().fg(Color::DarkGray),
    };

    // Show node details + output in right pane
    let selected_node_info = if app.discovered_nodes.is_empty() {
        "No nodes discovered yet.\n\nPress 'r' to rescan or make sure other nodes are running broadcast_existence().".to_string()
    } else if let Some(node) = app.discovered_nodes.get(app.selected_host) {
        format!(
            "Node:    {}\nAddress: {}\nPort:    {}\nCode:    {}\n\n{}",
            node.name, node.address, node.port, node.pairing_code, app.output
        )
    } else {
        app.output.clone()
    };

    let output = Paragraph::new(selected_node_info)
        .block(Block::default().title("Output").borders(Borders::ALL).border_style(output_border_style));

    frame.render_widget(output, content[1]);

    // --- Status bar ---
    let host_label = app.discovered_nodes.get(app.selected_host)
        .map(|h| h.name.clone())
        .unwrap_or_else(|| "none".to_string());

    let status = format!(
        " [←/→] switch node  [↑/↓] select task  [Enter] connect  [r] rescan  [Tab] focus  [q] quit  │  node: {} │  nodes: {}",
        host_label,
        app.discovered_nodes.len()
    );
    let status_bar = Paragraph::new(status)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status_bar, outer[2]);
}

fn main() -> io::Result<()> {
    // Load tasks from config
    let main_config = config::Config::new(Some("./despereaux.yaml".to_string())).unwrap();

    // Create node for discovery
    let node = Node::new("forsyth-tui".to_string());

    // Run initial scan
    println!("Running initial scan for 3 seconds...");
    let initial_nodes = node.find_connections(Duration::from_secs(3));
    println!("Found {} nodes", initial_nodes.len());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(node, main_config.data.tasks.clone(), initial_nodes);

    loop {
        terminal.draw(|frame| ui(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Char('r') => {
                        app.output = "Rescanning for nodes...".to_string();
                        app.rescan();
                        app.output = format!("Rescan complete. Found {} nodes.", app.discovered_nodes.len());
                    }
                    KeyCode::Right     => app.next_host(),
                    KeyCode::Left      => app.prev_host(),
                    KeyCode::Down      => app.next_task(),
                    KeyCode::Up        => app.prev_task(),
                    KeyCode::Tab => {
                        app.active_pane = match app.active_pane {
                            ActivePane::Tasks  => ActivePane::Output,
                            ActivePane::Output => ActivePane::Tasks,
                        };
                    }
                    KeyCode::Enter => {
                        if let Some(node) = app.discovered_nodes.get(app.selected_host) {
                            let task = &app.tasks[app.selected_task];
                            app.output = format!(
                                "Running '{}' on {}...\n\n(SSH not wired up yet)",
                                task, node.name
                            );
                            // Wire make_connection here when ready:
                            // app.node.make_connection(&node.name).ok();
                        } else {
                            app.output = "No node selected. Press 'r' to rescan.".to_string();
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit { break; }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}