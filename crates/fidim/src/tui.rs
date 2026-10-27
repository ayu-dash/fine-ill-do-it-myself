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
    /// Switch the default PHP version.
    UsePhp(String),
    /// Switch the chosen web server.
    UseWeb(String),
    /// Write one port value into etc/ports.toml.
    SetPort { key: String, value: u16 },
    /// Show detail view for a service.
    ShowDetail(String),
}

/// One row of the config panel.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigRow {
    /// `php.use <version>`
    UsePhp(String),
    /// `web.use <server>` (with probed binary version when installed)
    UseWeb { name: String, ver: Option<String> },
    /// `port <key> = <value>`
    Port { key: String, value: u16 },
}

/// Active bottom-bar editor when changing a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Port(String),
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
    !app.panel_right && app.cursor() < app.services.len() && want == recv
}

pub struct App {
    pub services: Vec<ServiceStatus>,
    pub projects: Vec<String>,
    pub config_rows: Vec<ConfigRow>,
    pub cursor_left: usize,
    pub cursor_right: usize,
    pub panel_right: bool, // false = left panel, true = right panel
    pub message: String,
    pub quit: bool,
    pub keybinds: Keybinds,
    pub actions: Vec<Action>,
    pub edit: Option<Edit>,
    pub input: String,
    pub detail: Option<String>, // detail view content
    #[allow(dead_code)]
    pub progress: Option<(u64, u64)>,
}

impl App {
    pub fn new(services: Vec<ServiceStatus>, projects: Vec<String>) -> Self {
        Self {
            services,
            projects,
            config_rows: Vec::new(),
            cursor_left: 0,
            cursor_right: 0,
            panel_right: false,
            message: String::new(),
            quit: false,
            keybinds: Keybinds::default(),
            actions: Vec::new(),
            edit: None,
            input: String::new(),
            detail: None,
            progress: None,
        }
    }

    fn cursor(&self) -> usize {
        if self.panel_right {
            self.cursor_right
        } else {
            self.cursor_left
        }
    }

    fn cursor_mut(&mut self) -> &mut usize {
        if self.panel_right {
            &mut self.cursor_right
        } else {
            &mut self.cursor_left
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
        // bottom-bar editor captures digits/backspace/enter/esc while active
        if self.edit.is_some() {
            match key {
                KeyCode::Enter => {
                    if let Some(Edit::Port(k)) = self.edit.take() {
                        match self.input.parse::<u16>() {
                            Ok(v) => {
                                self.message = format!("port {k} set to {v}");
                                self.actions.push(Action::SetPort { key: k, value: v });
                            }
                            Err(_) => {
                                self.message = format!("invalid port '{0}'", self.input);
                                if self.input.parse::<u16>().is_ok() {
                                    self.message.push_str(" (unreachable)");
                                }
                            }
                        }
                    }
                    self.input.clear();
                    return;
                }
                KeyCode::Esc => {
                    self.edit = None;
                    self.input.clear();
                    self.message = "edit cancelled".into();
                    return;
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    return;
                }
                KeyCode::Char(c) if c.is_ascii_digit() && self.input.len() < 5 => {
                    self.input.push(c);
                    return;
                }
                _ => return,
            }
        }
        if key == self.keybinds.quit {
            self.quit = true;
            return;
        }
        if key == KeyCode::Esc {
            if self.detail.is_some() {
                self.detail = None;
                self.panel_right = false;
                self.cursor_left = 0;
                return;
            }
            self.quit = true;
            return;
        }
        match key {
            KeyCode::Down | KeyCode::Char('j') => self.cursor_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.cursor_move(-1),
            KeyCode::Tab => {
                self.panel_right = !self.panel_right;
                self.clamp_cursor();
            }
            KeyCode::Char('d') => {
                if self.cursor() < self.services.len() && !self.panel_right {
                    let name = self.services[self.cursor()].name.clone();
                    self.detail = Some(format!("detail: {}", name));
                    self.panel_right = true;
                    self.cursor_right = 0;
                    self.actions.push(Action::ShowDetail(name));
                }
            }
            KeyCode::Enter => match focused_panel(self) {
                // services: toggle under cursor (start when stopped, stop when running)
                "services" => {
                    let i = self.cursor();
                    if self.services[i].state == ServiceState::Running {
                        self.message = "stop requested".into();
                        self.actions.push(Action::StopService(i));
                    } else {
                        self.message = "start requested".into();
                        self.actions.push(Action::StartService(i));
                    }
                }
                "config" => self.on_config_enter(),
                _ => {
                    self.message = "projects enter not implemented; use compose panel keys".into();
                }
            },
            _ if cursor_is_service_row(self, key, self.keybinds.start) => {
                self.message = "start requested".into();
                self.actions.push(Action::StartService(self.cursor()));
            }
            _ if cursor_is_service_row(self, key, self.keybinds.stop) => {
                self.message = "stop requested".into();
                self.actions.push(Action::StopService(self.cursor()));
            }
            _ if key == self.keybinds.restart => {
                if !self.panel_right && self.cursor() < self.services.len() {
                    self.message = "service restart requested".into();
                    self.actions.push(Action::RestartService(self.cursor()));
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

    fn on_config_enter(&mut self) {
        let ci = self
            .cursor()
            .saturating_sub(self.services.len())
            .saturating_sub(self.projects.len());
        match self.config_rows.get(ci).cloned() {
            Some(c @ ConfigRow::UsePhp(_)) => {
                let c = c.clone();
                if let ConfigRow::UsePhp(v) = c {
                    self.message = format!("php.use {v} requested");
                    self.actions.push(Action::UsePhp(v));
                }
            }
            Some(c @ ConfigRow::UseWeb { .. }) => {
                if let ConfigRow::UseWeb { name, .. } = c {
                    self.message = format!("web.use {name} requested");
                    self.actions.push(Action::UseWeb(name));
                }
            }
            Some(ConfigRow::Port { key, value }) => {
                self.edit = Some(Edit::Port(key.clone()));
                self.input = value.to_string();
                self.message = "Enter terapkan, Esc batal".into();
            }
            None => self.message = "no config row under cursor".into(),
        }
    }

    fn cursor_move(&mut self, by: i32) {
        let cur = self.cursor_mut();
        let c = *cur as i32 + by;
        *cur = c.max(0) as usize;
        self.clamp_cursor();
    }

    pub fn clamp_now(&mut self) {
        self.clamp_cursor()
    }

    fn clamp_cursor(&mut self) {
        // cursor walks only within current panel
        let len = if self.panel_right {
            self.config_rows.len().max(1)
        } else {
            self.services.len() + self.projects.len()
        };
        if len == 0 {
            *self.cursor_mut() = 0;
            return;
        }
        let cur = self.cursor_mut();
        *cur = (*cur).min(len - 1);
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
    if app.panel_right {
        "management"
    } else if app.cursor() < app.services.len() {
        "services"
    } else if app.cursor() < app.services.len() + app.projects.len() {
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
    } else if name == "caddy" || name == "httpd" || name == "nginx" || name == "lighttpd" {
        // each row shows its own per-server ports, not the shared pair
        let (http, https) = ports.web_ports(name);
        format!("http:{http} https:{https}")
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

                // Only show detail panel if app.detail is Some
                let (main_area, detail_area) = if app.detail.is_some() {
                    let [left, right] = Layout::horizontal([
                        Constraint::Percentage(50),
                        Constraint::Percentage(50),
                    ])
                    .areas(area);
                    (left, Some(right))
                } else {
                    (area, None)
                };

                let [services, projects, footer] = Layout::vertical([
                    Constraint::Length(app.services.len() as u16 + 2),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .areas(main_area);

                // services panel
                let items: Vec<ListItem> = app
                    .services
                    .iter()
                    .map(|s| {
                        let (name, mark, color) = match &s.state {
                            ServiceState::Running => {
                                ("● ".to_string(), s.name.clone(), Color::Green)
                            }
                            ServiceState::Error(msg) => {
                                ("✗ ".to_string(), format!("{} ({msg})", s.name), Color::Red)
                            }
                            _ => ("○ ".to_string(), s.name.clone(), Color::DarkGray),
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
                let svc_focus = !app.panel_right && app.cursor() < app.services.len();
                let proj_focus = !app.panel_right && app.cursor() >= app.services.len();
                let mut lst = ListState::default();
                if svc_focus {
                    lst.select(Some(app.cursor().min(app.services.len().saturating_sub(1))));
                }
                let svc_marker = if svc_focus { "▶ " } else { " " };
                let svc_list = List::new(items)
                    .highlight_symbol(svc_marker.to_owned())
                    .highlight_style(Style::default().bg(Color::Cyan))
                    .block(
                        Block::default()
                            .title(format!(
                                " services ({web_name}){} ",
                                if svc_focus { " ◀" } else { "" }
                            ))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(if svc_focus {
                                Color::Cyan
                            } else {
                                Color::DarkGray
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
                if proj_focus {
                    plst.select(Some(
                        app.cursor()
                            .saturating_sub(app.services.len())
                            .min(app.projects.len().saturating_sub(1)),
                    ));
                }
                let proj_list = List::new(proj_items)
                    .highlight_symbol(if proj_focus { "▶ " } else { " " }.to_owned())
                    .highlight_style(Style::default().bg(Color::Blue))
                    .block(
                        Block::default()
                            .title(format!(
                                " projects ({}: ~/Sites){} ",
                                app.projects.len(),
                                if proj_focus { " ◀" } else { "" }
                            ))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(if proj_focus {
                                Color::Blue
                            } else {
                                Color::DarkGray
                            })),
                    );
                f.render_stateful_widget(proj_list, projects, &mut plst);

                // detail panel - only shown when 'd' is pressed
                if let Some(detail_area) = detail_area {
                    let lock =
                        fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
                    let cur_php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
                    let cur_web = fine_ill_do_it_myself::orchestrator::web_server(env);

                    let mut mgmt_items: Vec<ListItem> = Vec::new();
                    let mut mgmt_title = " management ".to_string();
                    let mgmt_focus = app.panel_right;

                    // Determine what to show based on cursor position
                    if app.cursor_left < app.services.len() {
                        let svc_name = &app.services[app.cursor_left].name;

                        if svc_name.starts_with("php-") {
                            // Show PHP version management
                            mgmt_title = format!(" php versions (current: {cur_php}) ");
                            for v in
                                fine_ill_do_it_myself::orchestrator::installed_php_versions(env)
                            {
                                let mark = if v == cur_php { "▶ " } else { "  " };
                                mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                    format!("{mark}php.use {v}"),
                                    Style::default().fg(Color::Yellow),
                                ))));
                            }
                        } else if ["caddy", "httpd", "nginx", "lighttpd"]
                            .contains(&svc_name.as_str())
                        {
                            // Show web server management
                            let ver =
                                fine_ill_do_it_myself::orchestrator::web_version(env, svc_name);
                            let v = ver.as_deref().unwrap_or("not installed");
                            mgmt_title = format!(" web server [{v}] ");
                            for w in [
                                fine_ill_do_it_myself::orchestrator::CADDY,
                                fine_ill_do_it_myself::orchestrator::APACHE,
                                fine_ill_do_it_myself::orchestrator::NGINX,
                                fine_ill_do_it_myself::orchestrator::LIGHTTPD,
                            ] {
                                let mark = if w == cur_web { "▶ " } else { "  " };
                                let wv = fine_ill_do_it_myself::orchestrator::web_version(env, w);
                                let wv_txt = wv.as_deref().unwrap_or("not installed");
                                mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                    format!("{mark}web.use {w}  [{wv_txt}]"),
                                    Style::default().fg(Color::Cyan),
                                ))));
                            }
                        } else if svc_name == "mariadb" {
                            // Show port management
                            mgmt_title = " ports ".to_string();
                            let ports =
                                fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| {
                                    std::env::var(k).ok()
                                });
                            mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                format!("  port http = {}", ports.http),
                                Style::default().fg(Color::Yellow),
                            ))));
                            mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                format!("  port https = {}", ports.https),
                                Style::default().fg(Color::Yellow),
                            ))));
                            mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                format!("  port fpm_base = {}", ports.fpm_base),
                                Style::default().fg(Color::Yellow),
                            ))));
                            mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                format!("  port mariadb = {}", ports.mariadb),
                                Style::default().fg(Color::Yellow),
                            ))));
                        }
                    } else {
                        // Default: show all config options
                        mgmt_title = " config (select service) ".to_string();
                        for r in &app.config_rows {
                            let txt = match r {
                                ConfigRow::UsePhp(v) => {
                                    let mark = if *v == cur_php { "▶ " } else { "  " };
                                    format!("{mark}php.use {v}")
                                }
                                ConfigRow::UseWeb { name, ver } => {
                                    let mark = if name == &cur_web { "▶ " } else { "  " };
                                    let v = ver.as_deref().unwrap_or("not installed");
                                    format!("{mark}web.use {name}  [{v}]")
                                }
                                ConfigRow::Port { key, value } => {
                                    format!("  port {key} = {value}")
                                }
                            };
                            mgmt_items.push(ListItem::new(Line::from(Span::styled(
                                txt,
                                Style::default().fg(Color::Yellow),
                            ))));
                        }
                    }

                    let mut mlst = ListState::default();
                    if mgmt_focus {
                        mlst.select(Some(app.cursor().min(mgmt_items.len().saturating_sub(1))));
                    }
                    let mgmt_list = List::new(mgmt_items)
                        .highlight_symbol("▶ ")
                        .highlight_style(Style::default().bg(Color::Magenta))
                        .block(
                            Block::default()
                                .title(mgmt_title)
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(if mgmt_focus {
                                    Color::Magenta
                                } else {
                                    Color::DarkGray
                                })),
                        );
                    f.render_stateful_widget(mgmt_list, detail_area, &mut mlst);
                }

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
                        Style::default().fg(if app.message.to_lowercase().contains("error") {
                            Color::Red
                        } else {
                            Color::Yellow
                        }),
                    ));
                }
                if let Some(Edit::Port(key)) = &app.edit {
                    spans.push(Span::styled(
                        format!(" edit {key}: {}█ ", app.input),
                        Style::default().fg(Color::White).bg(Color::DarkGray),
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
                        refresh(&mut app);
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
        let ports =
            fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        let statuses = fine_ill_do_it_myself::orchestrator::status_all(
            &env,
            &ports,
            &fine_ill_do_it_myself::RealRunner,
        );
        app.services = statuses;

        // build config rows
        let mut rows: Vec<ConfigRow> = Vec::new();

        // php versions
        for v in fine_ill_do_it_myself::orchestrator::installed_php_versions(&env) {
            rows.push(ConfigRow::UsePhp(v));
        }

        // web servers
        for w in [
            fine_ill_do_it_myself::orchestrator::CADDY,
            fine_ill_do_it_myself::orchestrator::APACHE,
            fine_ill_do_it_myself::orchestrator::NGINX,
            fine_ill_do_it_myself::orchestrator::LIGHTTPD,
        ] {
            let ver = fine_ill_do_it_myself::orchestrator::web_version(&env, w);
            rows.push(ConfigRow::UseWeb {
                name: w.to_string(),
                ver,
            });
        }

        // ports
        rows.push(ConfigRow::Port {
            key: "http".into(),
            value: ports.http,
        });
        rows.push(ConfigRow::Port {
            key: "https".into(),
            value: ports.https,
        });
        rows.push(ConfigRow::Port {
            key: "fpm_base".into(),
            value: ports.fpm_base,
        });
        rows.push(ConfigRow::Port {
            key: "mariadb".into(),
            value: ports.mariadb,
        });

        app.config_rows = rows;
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
            Action::UsePhp(v) => {
                if fine_ill_do_it_myself::orchestrator::php_binary_for(&env, v).is_none() {
                    Err(fine_ill_do_it_myself::Error::PhpNotInstalled(v.clone()))
                } else if let Ok(mut lock) = fine_ill_do_it_myself::Lock::load(&env.lock_path()) {
                    lock.php_default = Some(v.clone());
                    let res: Result<Vec<_>, fine_ill_do_it_myself::Error> = lock
                        .save(&env.lock_path())
                        .map_err(|e| e.into())
                        .and_then(|_| {
                            fine_ill_do_it_myself::orchestrator::generate_configs(&env, &ports, v)
                        })
                        .map(|_| vec![]);
                    res
                } else {
                    Err(fine_ill_do_it_myself::Error::Other(
                        "failed to load lock".into(),
                    ))
                }
            }
            Action::UseWeb(name) => {
                if fine_ill_do_it_myself::orchestrator::resolve_web_bin(&env, name).is_err() {
                    Err(fine_ill_do_it_myself::Error::Other(format!(
                        "web server {name} not installed"
                    )))
                } else if let Ok(mut lock) = fine_ill_do_it_myself::Lock::load(&env.lock_path()) {
                    lock.web_default = Some(name.clone());
                    let php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
                    let res: Result<Vec<_>, fine_ill_do_it_myself::Error> = lock
                        .save(&env.lock_path())
                        .map_err(|e| e.into())
                        .and_then(|_| {
                            fine_ill_do_it_myself::orchestrator::generate_configs(
                                &env, &ports, &php,
                            )
                        })
                        .map(|_| vec![]);
                    res
                } else {
                    Err(fine_ill_do_it_myself::Error::Other(
                        "failed to load lock".into(),
                    ))
                }
            }
            Action::SetPort { key, value } => {
                let allowed = ["http", "https", "fpm_base", "mariadb"];
                if !allowed.contains(&key.as_str()) {
                    Ok(vec![])
                } else {
                    let file = env.paths.etc().join("ports.toml");
                    let mut lines: Vec<String> = std::fs::read_to_string(&file)
                        .unwrap_or_default()
                        .lines()
                        .map(String::from)
                        .filter(|l| !l.trim_start().starts_with(&format!("{key} =")))
                        .collect();
                    lines.push(format!("{key} = {value}"));
                    let _ = std::fs::create_dir_all(env.paths.etc());
                    std::fs::write(&file, lines.join("\n") + "\n")
                        .map(|_| vec![])
                        .map_err(|e| fine_ill_do_it_myself::Error::Other(e.to_string()))
                }
            }
            Action::ShowDetail(name) => {
                // Build detail content based on service
                let mut detail = format!("Service: {name}\n\n");
                if name.starts_with("php-") {
                    detail.push_str("PHP-FPM Service\n");
                    if let Ok(lock) = fine_ill_do_it_myself::Lock::load(&env.lock_path()) {
                        let cur = fine_ill_do_it_myself::orchestrator::default_php(&lock);
                        detail.push_str(&format!("Current default: {cur}\n"));
                    }
                    if let Some(ver) = name.strip_prefix("php-") {
                        if let Some(bin) =
                            fine_ill_do_it_myself::orchestrator::php_binary_for(&env, ver)
                        {
                            detail.push_str(&format!("Binary: {}\n", bin.display()));
                        }
                    }
                } else if ["caddy", "httpd", "nginx", "lighttpd"].contains(&name.as_str()) {
                    detail.push_str("Web Server\n");
                    if let Some(ver) = fine_ill_do_it_myself::orchestrator::web_version(&env, name)
                    {
                        detail.push_str(&format!("Version: {ver}\n"));
                    }
                    let ports = fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| {
                        std::env::var(k).ok()
                    });
                    let (http, https) = ports.web_ports(name.as_str());
                    detail.push_str(&format!("HTTP port: {http}\n"));
                    detail.push_str(&format!("HTTPS port: {https}\n"));
                } else if name == "mariadb" {
                    detail.push_str("MariaDB Database\n");
                    let ports = fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| {
                        std::env::var(k).ok()
                    });
                    detail.push_str(&format!("Port: {}\n", ports.mariadb));
                }
                app.detail = Some(detail);
                Ok(vec![])
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
        assert_eq!(a.cursor(), 2, "cursor walks past services into projects");
        a.on_key(KeyCode::Up);
        assert_eq!(a.cursor(), 1);
        a.on_key(KeyCode::Char('k'));
        assert_eq!(a.cursor(), 0);
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
        let ports = fine_ill_do_it_myself::PortPlan::default();
        // each web server row shows its OWN ports, never the shared pair
        assert_eq!(ports_label("caddy", "8.4", &ports), "http:8080 https:8443");
        assert_eq!(ports_label("httpd", "8.4", &ports), "http:8081 https:8444");
        assert_eq!(ports_label("nginx", "8.4", &ports), "http:8082 https:8445");
        assert_eq!(
            ports_label("lighttpd", "8.4", &ports),
            "http:8083 https:8446"
        );
        assert_eq!(ports_label("php-8.4", "8.4", &ports), "fpm 9004");
        assert_eq!(ports_label("php-8.4", "8.2", &ports), "");
        assert_eq!(ports_label("mariadb", "8.4", &ports), "3306");
    }

    #[test]
    fn ports_label_shows_overridden_ports_from_ports_toml() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("ports.toml"), "nginx_http = 9090\n").unwrap();
        let ports = fine_ill_do_it_myself::PortPlan::parse(
            &std::fs::read_to_string(t.path().join("ports.toml")).unwrap(),
        );
        assert_eq!(ports_label("nginx", "8.4", &ports), "http:9090 https:8445");
    }

    #[test]
    fn quick_create_plan_covers_dir_and_hosts() {
        let plan = quick_create_plan("~/Sites", "shop");
        assert_eq!(plan, vec!["mkdir -p ~/Sites/shop", "127.0.0.1 shop.test"]);
    }
}
