use fine_ill_do_it_myself::{ServiceState, ServiceStatus};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Restart every service (stop_all + start_all).
    RestartAll,
    /// Start the service under the cursor.
    StartService(usize),
    /// Stop the service under the cursor.
    StopService(usize),
    /// Restart the service under the cursor.
    RestartService(usize),
}

#[derive(Clone)]
pub struct Keybinds {
    pub quit: crossterm::event::KeyCode,
    /// per-service restart (r)
    pub restart: crossterm::event::KeyCode,
    /// per-service start (s)
    pub start: crossterm::event::KeyCode,
    /// per-service stop (x)
    pub stop: crossterm::event::KeyCode,
    /// restart everything (a)
    pub restart_all: crossterm::event::KeyCode,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            quit: crossterm::event::KeyCode::Char('q'),
            restart: crossterm::event::KeyCode::Char('r'),
            start: crossterm::event::KeyCode::Char('s'),
            stop: crossterm::event::KeyCode::Char('x'),
            restart_all: crossterm::event::KeyCode::Char('a'),
        }
    }
}

/// Parse `quit = q` style lines from the keybind config file.
#[allow(dead_code)]
pub fn load_keybinds(text: &str) -> Keybinds {
    let mut k = Keybinds::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some((key, val)) = line.split_once('=') {
            let val: String = val.trim().chars().collect();
            let code = parse_keycode(val.as_str()).unwrap_or(k.quit);
            match key.trim() {
                "quit" => k.quit = code,
                "restart" => k.restart = code,
                "start" => k.start = code,
                "stop" => k.stop = code,
                "restart_all" => k.restart_all = code,
                _ => {}
            }
        }
    }
    k
}

#[allow(dead_code)]
fn parse_keycode(v: &str) -> Option<crossterm::event::KeyCode> {
    let mut chars = v.chars();
    match (chars.next(), chars.count()) {
        (Some(c), 0) => Some(crossterm::event::KeyCode::Char(c)),
        _ => None,
    }
}

/// Simple progress bar for downloads.
#[allow(dead_code)]
pub fn progress_bar(done: u64, total: u64, width: usize) -> String {
    let pct = if total == 0 {
        0.0
    } else {
        done as f64 / total as f64
    };
    let filled = (pct * width as f64) as usize;
    let mut s = String::from("[");
    for i in 0..width {
        s.push(if i < filled { '#' } else { '.' });
    }
    let p = (pct * 100.0) as u64;
    s.push_str(&format!("] {:>3}%", p));
    s
}

/// True when the cursor sits on a service row and the key matches the binding.
pub fn cursor_is_service_row(
    app: &App,
    want: crossterm::event::KeyCode,
    recv: crossterm::event::KeyCode,
) -> bool {
    app.cursor < app.services.len() && want == recv
}

pub struct App {
    pub services: Vec<ServiceStatus>,
    pub projects: Vec<String>,
    pub cursor: usize,
    pub message: String,
    pub quit: bool,
    pub keybinds: Keybinds,
    pub actions: Vec<Action>,
    #[allow(dead_code)]
    pub progress: Option<(u64, u64)>,
}

impl App {
    pub fn new(services: Vec<ServiceStatus>, projects: Vec<String>) -> Self {
        Self {
            services,
            projects,
            cursor: 0,
            message: String::new(),
            quit: false,
            keybinds: Keybinds::default(),
            actions: Vec::new(),
            progress: None,
        }
    }

    #[allow(dead_code)]
    pub fn running_count(&self) -> usize {
        self.services
            .iter()
            .filter(|s| s.state == ServiceState::Running)
            .count()
    }

    /// Pure transition for a key event.
    pub fn on_key(&mut self, key: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        if key == self.keybinds.quit || key == KeyCode::Esc {
            self.quit = true;
            return;
        }
        match key {
            KeyCode::Down | KeyCode::Char('j') => self.cursor_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.cursor_move(-1),
            KeyCode::Tab => {
                self.cursor = self.cursor.saturating_add(1);
                self.clamp_cursor();
            }
            _ if cursor_is_service_row(self, key, self.keybinds.start) => {
                self.message = "start requested".into();
                self.actions.push(Action::StartService(self.cursor));
            }
            _ if cursor_is_service_row(self, key, self.keybinds.stop) => {
                self.message = "stop requested".into();
                self.actions.push(Action::StopService(self.cursor));
            }
            _ if key == self.keybinds.restart => {
                if self.cursor < self.services.len() {
                    self.message = "service restart requested".into();
                    self.actions.push(Action::RestartService(self.cursor));
                } else {
                    self.message = "stop+start all requested".into();
                    self.actions.push(Action::RestartAll);
                }
            }
            _ if key == self.keybinds.restart_all => {
                self.message = "stop+start all requested".into();
                self.actions.push(Action::RestartAll);
            }
            _ => {}
        }
    }

    fn cursor_move(&mut self, by: i32) {
        let c = self.cursor as i32 + by;
        self.cursor = c.max(0) as usize;
        self.clamp_cursor();
    }

    pub fn clamp_now(&mut self) {
        self.clamp_cursor()
    }

    fn clamp_cursor(&mut self) {
        // cursor walks services first, then projects
        let len = self.services.len() + self.projects.len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(len - 1);
    }
}

/// Render the app state into a plain multi-line preview (testable; ratatui
/// widgets use the same data in `draw`).
#[allow(dead_code)]
pub fn preview(app: &App) -> String {
    let mut out = String::from("fine-ill-do-it-myself\n");
    out.push_str(&format!("services running: {}\n", app.running_count()));
    for s in &app.services {
        out.push_str(&format!("  {:<12} {:?}\n", s.name, s.state));
    }
    out.push_str("projects:\n");
    for p in &app.projects {
        out.push_str(&format!("  {p}\n"));
    }
    if let Some((d, t)) = app.progress {
        out.push_str(&format!("{}\n", progress_bar(d, t, 20)));
    }
    out.push_str("press q to quit\n");
    out
}

/// Last n lines of a log file (missing file = empty).
#[allow(dead_code)]
pub fn tail(path: &std::path::Path, n: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let collected: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let start = collected.len().saturating_sub(n);
    collected[start..].to_vec()
}

/// Compose the quick-create plan: a project folder to mkdir + hosts entry.
#[allow(dead_code)]
pub fn quick_create_plan(sites: &str, name: &str) -> Vec<String> {
    let mut plan = Vec::new();
    plan.push(format!("mkdir -p {sites}/{name}"));
    plan.push(format!("127.0.0.1 {name}.test"));
    plan
}

/// Which panel is focused given a cursor that walks services then projects.
pub fn focused_panel(app: &App) -> &'static str {
    if app.cursor < app.services.len() {
        "services"
    } else if app.cursor < app.services.len() + app.projects.len() {
        "projects"
    } else {
        "services"
    }
}

/// Port label per service row, shown beside its name.
pub fn ports_label(name: &str, php_row: &str, ports: &fine_ill_do_it_myself::PortPlan) -> String {
    if name == "mariadb" {
        "3306".to_string()
    } else if let Some(ver) = name.strip_prefix("php-") {
        if php_row != ver {
            return String::new();
        }
        format!(
            "fpm {}",
            fine_ill_do_it_myself::php::fpm::fpm_port_for(ver, 9000, ports)
        )
    } else if name == "caddy" || name == "httpd" || name == "nginx" {
        format!("http:{} https:{}", ports.http, ports.https)
    } else {
        String::new()
    }
}

/// Terminal entry: raw mode loop with ratatui rendering of status + logs.
pub fn run_tui(
    env: &fine_ill_do_it_myself::Env,
    ports: fine_ill_do_it_myself::PortPlan,
) -> Result<(), fine_ill_do_it_myself::Error> {
    use crossterm::event::{Event, KeyEventKind};
    use crossterm::terminal;
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    // refuse to hijack a non-terminal; scripted invocations keep working
    if unsafe { libc::isatty(1) } != 1 {
        println!("TUI requires a tty; use: status");
        return Ok(());
    }
    let refresh = make_refresh(env.clone());
    let act = make_actioner(env.clone(), ports.clone());
    let _ = terminal::enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), terminal::EnterAlternateScreen);

    let projects: Vec<String> = fine_ill_do_it_myself::list_projects(env);
    let mut app = App::new(Vec::new(), projects);
    app.keybinds = load_keybinds(
        &std::fs::read_to_string(env.paths.etc().join("keys.toml")).unwrap_or_default(),
    );

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend).ok();
    refresh(&mut app);
    loop {
        if let Some(t) = term.as_mut() {
            let _ = t.draw(|f| {
                use ratatui::layout::{Constraint, Layout};
                use ratatui::style::{Color, Modifier, Style};
                use ratatui::text::{Line, Span};
                use ratatui::widgets::{
                    Block, BorderType, Borders, List, ListItem, ListState, Paragraph,
                };

                let area = f.area();
                let [services, projects, footer] = Layout::vertical([
                    Constraint::Length(app.services.len() as u16 + 2),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .areas(area);

                // services panel
                let items: Vec<ListItem> = app
                    .services
                    .iter()
                    .map(|s| {
                        let (name, mark, color) = match s.state {
                            ServiceState::Running => ("● ", s.name.as_str(), Color::Green),
                            _ => ("○ ", s.name.as_str(), Color::DarkGray),
                        };
                        let lock =
                            fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
                        let php_row = fine_ill_do_it_myself::orchestrator::default_php(&lock);
                        let p = ports_label(&s.name, &php_row, &ports);
                        ListItem::new(Line::from(vec![
                            Span::styled(mark, Style::default().fg(color)),
                            Span::styled(
                                name.to_string(),
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(format!("  {p}"), Style::default().fg(Color::DarkGray)),
                        ]))
                    })
                    .collect();
                let web_name = fine_ill_do_it_myself::orchestrator::web_server(env);
                let svc_focus = focused_panel(&app) == "projects";
                let mut lst = ListState::default();
                lst.select(Some(app.cursor.min(app.services.len().saturating_sub(1))));
                let svc_marker = if svc_focus { " " } else { "▶ " };
                let svc_list = List::new(items)
                    .highlight_symbol(svc_marker.to_owned())
                    .highlight_style(Style::default().bg(Color::Cyan))
                    .block(
                        Block::default()
                            .title(format!(
                                " services ({web_name}){} ",
                                if svc_focus { "" } else { " ◀" }
                            ))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(if svc_focus {
                                Color::DarkGray
                            } else {
                                Color::Cyan
                            })),
                    );
                f.render_stateful_widget(svc_list, services, &mut lst);

                // projects panel
                let proj_items: Vec<ListItem> = app
                    .projects
                    .iter()
                    .map(|p| {
                        ListItem::new(Line::from(vec![
                            Span::styled("▸ ", Style::default().fg(Color::Cyan)),
                            Span::styled(p.clone(), Style::default().fg(Color::White)),
                            Span::styled(".test  ", Style::default().fg(Color::DarkGray)),
                        ]))
                    })
                    .collect();
                let mut plst = ListState::default();
                plst.select(Some(
                    app.cursor
                        .saturating_sub(app.services.len())
                        .min(app.projects.len().saturating_sub(1)),
                ));
                let proj_list = List::new(proj_items)
                    .highlight_symbol(if svc_focus { "▶ " } else { " " }.to_owned())
                    .highlight_style(Style::default().bg(Color::Blue))
                    .block(
                        Block::default()
                            .title(format!(
                                " projects ({}: ~/Sites){} ",
                                app.projects.len(),
                                if svc_focus { " ◀" } else { "" }
                            ))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(if svc_focus {
                                Color::Blue
                            } else {
                                Color::DarkGray
                            })),
                    );
                f.render_stateful_widget(proj_list, projects, &mut plst);

                // footer with key hints
                let hints = vec![
                    (app.keybinds.quit, "quit"),
                    (crossterm::event::KeyCode::Char('j'), "down"),
                    (crossterm::event::KeyCode::Char('k'), "up"),
                    (app.keybinds.start, "start"),
                    (app.keybinds.stop, "stop"),
                    (app.keybinds.restart, "restart"),
                    (app.keybinds.restart_all, "all"),
                ];
                let mut spans: Vec<Span> = Vec::new();
                for (code, label) in hints {
                    let key = match code {
                        crossterm::event::KeyCode::Char(c) => format!("{c}"),
                        _ => format!("{code:?}"),
                    };
                    spans.push(Span::styled(
                        format!(" {key} "),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::styled(
                        format!(" {label} "),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                if let Some((d, tot)) = app.progress {
                    let pct = (d * 100).checked_div(tot).unwrap_or(0);
                    spans.push(Span::styled(
                        format!(" {pct}% "),
                        Style::default().fg(Color::Green),
                    ));
                }
                if !app.message.is_empty() {
                    spans.push(Span::styled(
                        format!(" {} ", app.message),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                f.render_widget(Paragraph::new(Line::from(spans)), footer);
            });
        }
        if crossterm::event::poll(std::time::Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = crossterm::event::read() {
                if k.kind == KeyEventKind::Press {
                    app.on_key(k.code);
                    while let Some(a) = app.actions.pop() {
                        act(&mut app, &a);
                    }
                }
            }
        }
        if app.quit {
            break;
        }
    }

    let _ = crossterm::execute!(std::io::stdout(), terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    Ok(())
}

fn make_refresh(env: fine_ill_do_it_myself::Env) -> impl Fn(&mut App) {
    move |app: &mut App| {
        let statuses = fine_ill_do_it_myself::orchestrator::status_all(
            &env,
            &fine_ill_do_it_myself::PortPlan::default(),
            &fine_ill_do_it_myself::RealRunner,
        );
        app.services = statuses;
        app.clamp_now();
    }
}

/// Execute one queued action against the real system; sets the status-bar
/// message with the outcome.
fn make_actioner(
    env: fine_ill_do_it_myself::Env,
    ports: fine_ill_do_it_myself::PortPlan,
) -> impl Fn(&mut App, &Action) {
    move |app: &mut App, action: &Action| {
        let runner = fine_ill_do_it_myself::RealRunner;
        let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
        let php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
        let r: Result<Vec<_>, _> = match action {
            Action::RestartAll => {
                fine_ill_do_it_myself::orchestrator::stop_all(&env, &ports, &runner).and_then(
                    |_| fine_ill_do_it_myself::orchestrator::start_all(&env, &ports, &runner, &php),
                )
            }
            Action::StartService(i) => {
                let name = app.services[*i].name.clone();
                fine_ill_do_it_myself::orchestrator::start_service(
                    &env, &ports, &runner, &php, &name,
                )
                .map(|s| vec![s])
            }
            Action::StopService(i) => {
                let name = app.services[*i].name.clone();
                let sup = fine_ill_do_it_myself::supervisor::Supervisor {
                    env: env.clone(),
                    ports: ports.clone(),
                    runner: runner.clone(),
                };
                sup.stop(&name).map(|_| Vec::new())
            }
            Action::RestartService(i) => {
                let name = app.services[*i].name.clone();
                let sup = fine_ill_do_it_myself::supervisor::Supervisor {
                    env: env.clone(),
                    ports: ports.clone(),
                    runner: runner.clone(),
                };
                sup.stop(&name).and_then(|_| {
                    fine_ill_do_it_myself::orchestrator::start_service(
                        &env, &ports, &runner, &php, &name,
                    )
                    .map(|s| vec![s])
                })
            }
        };
        match r {
            Ok(_) => app.message = format!("{action:?} ok").to_lowercase(),
            Err(e) => app.message = format!("{action:?}: {e}").to_lowercase(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn svc(name: &str, running: bool) -> ServiceStatus {
        ServiceStatus {
            name: name.into(),
            state: if running {
                ServiceState::Running
            } else {
                ServiceState::Stopped
            },
            pid: Some(1),
        }
    }

    #[test]
    fn counts_running_services() {
        let a = App::new(vec![svc("caddy", true), svc("php-8.4", false)], vec![]);
        assert_eq!(a.running_count(), 1);
    }

    #[test]
    fn keys_config_overrides_defaults() {
        let k = load_keybinds("quit = z\nrestart = a\n");
        assert_eq!(k.quit, KeyCode::Char('z'));
        assert_eq!(k.restart, KeyCode::Char('a'));
    }

    #[test]
    fn keys_config_missing_lines_keep_defaults() {
        let k = load_keybinds("");
        assert_eq!(k.quit, KeyCode::Char('q'));
        assert_eq!(k.restart, KeyCode::Char('r'));
    }

    #[test]
    fn quit_key_sets_flag() {
        let mut a = App::new(vec![], vec![]);
        a.on_key(KeyCode::Char('q'));
        assert!(a.quit);
        let mut b = App::new(vec![], vec![]);
        b.keybinds.quit = KeyCode::Char('z');
        b.on_key(KeyCode::Char('q'));
        b.on_key(KeyCode::Char('z'));
        assert!(b.quit);
    }

    #[test]
    fn arrow_keys_move_cursor_with_clamp() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("php-8.4", false)],
            vec!["blog".into()],
        );
        a.on_key(KeyCode::Down);
        a.on_key(KeyCode::Down);
        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor, 2, "cursor walks past services into projects");
        a.on_key(KeyCode::Up);
        assert_eq!(a.cursor, 1);
        a.on_key(KeyCode::Char('k'));
        assert_eq!(a.cursor, 0);
    }

    #[test]
    fn restart_key_emits_action() {
        let mut a = App::new(vec![], vec![]);
        a.on_key(KeyCode::Char('r'));
        assert!(!a.quit);
        assert!(a.message.contains("requested"));
        assert_eq!(a.actions[0], Action::RestartAll);
    }

    #[test]
    fn preview_lists_services_and_projects() {
        let a = App::new(
            vec![svc("caddy", true)],
            vec!["hello".into(), "blog".into()],
        );
        let p = preview(&a);
        assert!(p.contains("caddy"));
        assert!(p.contains("hello"));
        assert!(p.contains("blog"));
    }

    #[test]
    fn progress_bar_shows_decay_states() {
        assert_eq!(progress_bar(0, 100, 10), "[..........]   0%");
        assert_eq!(progress_bar(50, 100, 10), "[#####.....]  50%");
        assert_eq!(progress_bar(100, 100, 10), "[##########] 100%");
        assert_eq!(progress_bar(5, 0, 4), "[....]   0%");
    }

    #[test]
    fn tail_returns_last_n_lines() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.log");
        std::fs::write(&f, "1\n2\n3\n4\n").unwrap();
        assert_eq!(tail(&f, 2), vec!["3".to_string(), "4".to_string()]);
        assert!(tail(&t.path().join("missing"), 2).is_empty());
    }

    #[test]
    fn keys_config_per_service_overrides() {
        let k = load_keybinds("quit = z\nstart = b\nstop = n\nrestart = t\nrestart_all = g\n");
        assert_eq!(k.start, KeyCode::Char('b'));
        assert_eq!(k.stop, KeyCode::Char('n'));
        assert_eq!(k.restart, KeyCode::Char('t'));
        assert_eq!(k.restart_all, KeyCode::Char('g'));
    }

    #[test]
    fn service_row_keys_queue_actions() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("php-8.4", true)],
            vec!["blog".into()],
        );
        a.on_key(KeyCode::Char('s'));
        assert_eq!(a.actions[0], Action::StartService(0));
        a.actions.clear();
        a.on_key(KeyCode::Down);
        a.on_key(KeyCode::Char('x'));
        assert_eq!(a.actions[0], Action::StopService(1));
        a.actions.clear();
        a.on_key(KeyCode::Char('r'));
        assert_eq!(a.actions[0], Action::RestartService(1));
        // on the projects panel `r` restarts everything
        a.actions.clear();
        a.on_key(KeyCode::Down);
        a.on_key(KeyCode::Char('r'));
        assert_eq!(a.actions[0], Action::RestartAll);
    }

    #[test]
    fn focused_panel_follows_cursor_into_projects() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        assert_eq!(focused_panel(&a), "services");
        a.on_key(KeyCode::Down);
        assert_eq!(focused_panel(&a), "projects");
        a.on_key(KeyCode::Up);
        assert_eq!(focused_panel(&a), "services");
    }

    #[test]
    fn ports_label_per_service() {
        let ports = fine_ill_do_it_myself::PortPlan {
            http: 8080,
            https: 8443,
        };
        assert_eq!(ports_label("caddy", "8.4", &ports), "http:8080 https:8443");
        assert_eq!(ports_label("httpd", "8.4", &ports), "http:8080 https:8443");
        assert_eq!(ports_label("php-8.4", "8.4", &ports), "fpm 9004");
        assert_eq!(ports_label("php-8.4", "8.2", &ports), "");
        assert_eq!(ports_label("mariadb", "8.4", &ports), "3306");
    }

    #[test]
    fn quick_create_plan_covers_dir_and_hosts() {
        let plan = quick_create_plan("~/Sites", "shop");
        assert_eq!(plan, vec!["mkdir -p ~/Sites/shop", "127.0.0.1 shop.test"]);
    }
}
