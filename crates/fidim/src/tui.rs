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
    /// Switch a web server's active binary version (e.g. caddy -> caddy-2.9.1).
    UseServerVersion { server: String, version: String },
    /// Switch a programming language version (e.g. node -> node-22).
    UseLang { lang: String, version: String },
    /// Open a project in the default browser.
    OpenProject(String),
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

/// True when the cursor sits on a service row (not a language runtime) and the
/// key matches the binding. `node`/`python`/`go`/`java` are runtimes, not
/// long-running services, so they have nothing to start or stop.
pub fn cursor_is_service_row(
    app: &App,
    want: crossterm::event::KeyCode,
    recv: crossterm::event::KeyCode,
) -> bool {
    !app.panel_right
        && app.cursor() < app.services.len()
        && !is_runtime_only(&app.services[app.cursor()].name)
        && want == recv
}

/// Language runtimes that are not services: everything except php-fpm.
fn is_runtime_only(name: &str) -> bool {
    is_language(name) && !name.starts_with("php")
}

/// True if a service is a programming language runtime (e.g. php-8.4).
pub fn is_language(name: &str) -> bool {
    name.starts_with("php-")
        || name == "php"
        || name.starts_with("node-")
        || name == "node"
        || name.starts_with("python-")
        || name == "python"
        || name.starts_with("ruby-")
        || name == "ruby"
        || name.starts_with("go-")
        || name == "go"
        || name.starts_with("java-")
        || name == "java"
}

/// Language family for a service row: `php-8.4` -> `php`, `node-24` -> `node`.
fn language_family(name: &str) -> &str {
    name.split('-').next().unwrap_or(name)
}

/// First service index of every language family. Used before the first refresh
/// (which replaces this with the active version of each language).
fn first_language_reps(services: &[ServiceStatus]) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut reps: Vec<usize> = services
        .iter()
        .enumerate()
        .filter(|(_, s)| is_language(&s.name))
        .filter(|(_, s)| seen.insert(language_family(&s.name).to_string()))
        .map(|(i, _)| i)
        .collect();
    reps.sort_unstable();
    reps
}

/// Representative row per language family using the active version from
/// lock.toml, matching the collapsed rows rendered in the left panel.
fn language_representatives(
    env: &fine_ill_do_it_myself::Env,
    services: &[ServiceStatus],
) -> Vec<usize> {
    let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
    let php_row = fine_ill_do_it_myself::orchestrator::default_php(&lock);
    let mut seen = std::collections::HashSet::new();
    let mut reps = Vec::new();
    for (idx, s) in services.iter().enumerate() {
        if !is_language(&s.name) {
            continue;
        }
        let base = language_family(&s.name);
        if !seen.insert(base.to_string()) {
            continue;
        }
        let target = if base == "php" {
            format!("php-{php_row}")
        } else {
            match fine_ill_do_it_myself::lang::active_lang_version(env, base) {
                Some(v) => format!("{base}-{v}"),
                None => base.to_string(),
            }
        };
        let rep = services
            .iter()
            .position(|x| x.name == target)
            .unwrap_or(idx);
        reps.push(rep);
    }
    reps.sort_unstable();
    reps.dedup();
    reps
}

/// Focusable regions of the main screen. `Tab`/`Shift+Tab` cycle through them;
/// the arrow keys and `j`/`k` never leave the active one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Container {
    Services,
    Languages,
    Projects,
    Management,
}

/// Cycle order used by `Tab` (and reversed by `Shift+Tab`).
const CONTAINERS: [Container; 4] = [
    Container::Services,
    Container::Languages,
    Container::Projects,
    Container::Management,
];

pub struct App {
    pub services: Vec<ServiceStatus>,
    pub projects: Vec<String>,
    pub config_rows: Vec<ConfigRow>,
    /// One representative service index per language family (php, node, ...).
    /// The left panel collapses all installed versions into these rows, so
    /// navigation must walk them instead of every hidden version.
    language_reps: Vec<usize>,
    pub cursor_left: usize,
    pub cursor_right: usize,
    pub panel_right: bool, // false = left panel, true = right panel
    pub message: String,
    pub quit: bool,
    pub keybinds: Keybinds,
    pub actions: Vec<Action>,
    pub edit: Option<Edit>,
    pub input: String,
    /// false while `input` still holds the untouched prefill of the editor
    pub edit_dirty: bool,
    pub detail: Option<String>, // detail view content
    #[allow(dead_code)]
    pub progress: Option<(u64, u64)>,
    /// Cached line count of the right management panel (updated each render
    /// cycle and before Enter dispatch so the cursor clamp is accurate).
    pub mgmt_line_count: usize,
    /// Cached management panel lines for cursor-skip logic.
    pub mgmt_lines: Vec<String>,
}

impl App {
    pub fn new(mut services: Vec<ServiceStatus>, projects: Vec<String>) -> Self {
        services.sort_by_key(|s| is_language(&s.name));
        let language_reps = first_language_reps(&services);
        Self {
            services,
            projects,
            config_rows: Vec::new(),
            language_reps,
            cursor_left: 0,
            cursor_right: 0,
            panel_right: false,
            message: String::new(),
            quit: false,
            keybinds: Keybinds::default(),
            actions: Vec::new(),
            edit: None,
            input: String::new(),
            edit_dirty: false,
            detail: None,
            progress: None,
            mgmt_line_count: 0,
            mgmt_lines: Vec::new(),
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
                            }
                        }
                    }
                    self.input.clear();
                    return;
                }
                KeyCode::Esc => {
                    self.edit = None;
                    self.input.clear();
                    self.edit_dirty = false;
                    self.message = "edit cancelled".into();
                    return;
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.edit_dirty = true;
                    return;
                }
                KeyCode::Char(c) if c.is_ascii_digit() && self.input.len() < 5 => {
                    // first keystroke replaces the prefilled value instead of
                    // being appended to it (e.g. "8082" + "9090" -> "9090")
                    if !self.edit_dirty {
                        self.input.clear();
                        self.edit_dirty = true;
                    }
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
            KeyCode::Tab => self.cycle_container(1),
            KeyCode::BackTab => self.cycle_container(-1),
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
                // languages: php-fpm is a service and toggles; the other
                // runtimes have no process to stop, so Enter opens their
                // version list instead.
                "languages" => {
                    let i = self.cursor();
                    let name = self.services[i].name.clone();
                    if is_runtime_only(&name) {
                        self.detail = Some(format!("detail: {name}"));
                        self.panel_right = true;
                        self.cursor_right = 0;
                        self.message = "select a version, Enter to activate".into();
                    } else if self.services[i].state == ServiceState::Running {
                        self.message = "stop requested".into();
                        self.actions.push(Action::StopService(i));
                    } else {
                        self.message = "start requested".into();
                        self.actions.push(Action::StartService(i));
                    }
                }
                "projects" => {
                    let proj_idx = self.cursor().saturating_sub(self.services.len());
                    if let Some(p) = self.projects.get(proj_idx).cloned() {
                        self.message = format!("opening {p}.test in browser...");
                        self.actions.push(Action::OpenProject(p));
                    }
                }
                _ => {
                    self.message = "enter not implemented; use compose panel keys".into();
                }
            },
            _ if !self.panel_right
                && self.cursor() < self.services.len()
                && is_runtime_only(&self.services[self.cursor()].name)
                && [
                    self.keybinds.start,
                    self.keybinds.stop,
                    self.keybinds.restart,
                ]
                .contains(&key) =>
            {
                self.message = "runtime selected: Enter opens its versions".into();
            }
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

    /// Enter on the right panel: act on the highlighted management line.
    /// `web.use X` switches the default server, `php.use X` the php default,
    /// `port KEY = V` opens the port editor prefilled with V (web-server
    /// rows map the generic `http`/`https` labels to their per-server keys).
    pub fn mgmt_enter(&mut self, lines: &[String]) {
        let sel = self.cursor().min(lines.len().saturating_sub(1));
        let line = lines.get(sel).cloned().unwrap_or_default();
        let text = line.trim().trim_start_matches("▶ ").trim();
        if let Some(v) = text.strip_prefix("php.use ") {
            self.message = format!("php.use {v} requested");
            self.actions.push(Action::UsePhp(v.to_string()));
        } else if let Some(n) = text.strip_prefix("web.use ") {
            self.message = format!("web.use {n} requested");
            self.actions.push(Action::UseWeb(n.to_string()));
        } else if let Some((lang_or_srv, ver)) = text.split_once(".use ") {
            let s = lang_or_srv.trim_start_matches("▶ ").trim();
            if ["node", "python", "go", "java"].contains(&s) {
                let target_ver = if ver.starts_with("system") {
                    "system"
                } else {
                    ver
                };
                self.message = format!("{s}.use {target_ver} requested");
                self.actions.push(Action::UseLang {
                    lang: s.to_string(),
                    version: target_ver.to_string(),
                });
            } else if [
                fine_ill_do_it_myself::orchestrator::CADDY,
                fine_ill_do_it_myself::orchestrator::APACHE,
                fine_ill_do_it_myself::orchestrator::NGINX,
                fine_ill_do_it_myself::orchestrator::LIGHTTPD,
            ]
            .contains(&s)
            {
                self.message = format!("{s}.use {ver} requested");
                self.actions.push(Action::UseServerVersion {
                    server: s.to_string(),
                    version: ver.to_string(),
                });
            }
        } else if let Some(rest) = text.strip_prefix("port ") {
            if let Some((label, value)) = rest.split_once(" = ") {
                // a web-server row applies its own pair, others the global one
                let key = match self.services.get(self.cursor_left).map(|s| s.name.as_str()) {
                    Some(n)
                        if [
                            fine_ill_do_it_myself::orchestrator::CADDY,
                            fine_ill_do_it_myself::orchestrator::APACHE,
                            fine_ill_do_it_myself::orchestrator::NGINX,
                            fine_ill_do_it_myself::orchestrator::LIGHTTPD,
                        ]
                        .contains(&n) =>
                    {
                        format!("{n}_{label}")
                    }
                    _ => label.to_string(),
                };
                self.edit = Some(Edit::Port(key));
                self.input = value.to_string();
                self.edit_dirty = false;
                self.message = "Enter terapkan, Esc batal".into();
            }
        }
    }

    /// Which container currently holds the cursor.
    fn container(&self) -> Container {
        if self.panel_right {
            return Container::Management;
        }
        if self.cursor_left < self.services.len() {
            if is_language(&self.services[self.cursor_left].name) {
                Container::Languages
            } else {
                Container::Services
            }
        } else {
            Container::Projects
        }
    }

    /// Visible row indices of one container. `Management` has no left rows.
    fn container_rows(&self, container: Container) -> Vec<usize> {
        match container {
            Container::Services => (0..self.services.len())
                .filter(|&i| !is_language(&self.services[i].name))
                .collect(),
            Container::Languages => self
                .language_reps
                .iter()
                .copied()
                .filter(|&i| i < self.services.len())
                .collect(),
            Container::Projects => {
                (self.services.len()..self.services.len() + self.projects.len()).collect()
            }
            Container::Management => Vec::new(),
        }
    }

    /// Give the focus to `container`, landing on its first row.
    fn focus_container(&mut self, container: Container) {
        match container {
            Container::Management => {
                self.panel_right = true;
                self.cursor_right = 0;
                self.clamp_cursor();
                self.skip_unselectable(1);
            }
            other => {
                self.panel_right = false;
                if let Some(&row) = self.container_rows(other).first() {
                    self.cursor_left = row;
                }
                self.normalize_left_cursor();
            }
        }
    }

    /// `Tab`/`Shift+Tab`: move the focus to the next/previous container,
    /// skipping the ones that have no rows. Only these keys change containers;
    /// the arrows stay inside the active one.
    fn cycle_container(&mut self, direction: i32) {
        let available: Vec<Container> = CONTAINERS
            .iter()
            .copied()
            .filter(|c| *c == Container::Management || !self.container_rows(*c).is_empty())
            .collect();
        if available.is_empty() {
            return;
        }
        let current = self.container();
        let index = available.iter().position(|c| *c == current).unwrap_or(0) as i32;
        let next = (index + direction).rem_euclid(available.len() as i32) as usize;
        self.focus_container(available[next]);
    }

    /// Move a cursor that landed on a hidden language version to that family's
    /// visible representative row.
    fn normalize_left_cursor(&mut self) {
        if self.panel_right
            || self.cursor_left >= self.services.len()
            || !is_language(&self.services[self.cursor_left].name)
            || self.language_reps.contains(&self.cursor_left)
        {
            return;
        }
        let family = language_family(&self.services[self.cursor_left].name).to_string();
        if let Some(rep) = self.language_reps.iter().copied().find(|&i| {
            self.services
                .get(i)
                .map(|s| language_family(&s.name) == family.as_str())
                .unwrap_or(false)
        }) {
            self.cursor_left = rep;
        }
    }

    fn cursor_move(&mut self, by: i32) {
        if !self.panel_right {
            let rows = self.container_rows(self.container());
            if rows.is_empty() {
                return;
            }
            let pos = rows
                .iter()
                .position(|&r| r == self.cursor_left)
                .unwrap_or(0) as i32;
            let next = (pos + by).clamp(0, rows.len() as i32 - 1) as usize;
            self.cursor_left = rows[next];
            return;
        }
        let cur = self.cursor_mut();
        let c = *cur as i32 + by;
        *cur = c.max(0) as usize;
        self.clamp_cursor();
        // On the right panel, skip non-selectable lines (empty / header).
        if self.detail.is_some() {
            self.skip_unselectable(by);
        }
    }

    /// Check whether a management panel line is selectable (non-empty and not
    /// a header like "  caddy versions:").
    fn is_mgmt_selectable(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty() && !t.ends_with(':')
    }

    /// After a cursor move on the right panel, skip over non-selectable lines
    /// (empty lines and headers).  `direction` is the original move (+1 / -1).
    fn skip_unselectable(&mut self, direction: i32) {
        let len = self.right_panel_len();
        if len == 0 {
            return;
        }
        let forward = direction >= 0;
        let mut pos = self.cursor_right;
        // Check cached lines; if unavailable, bail.
        if self.mgmt_lines.is_empty() {
            return;
        }
        // Try to find the next selectable line in the given direction.
        let mut attempts = 0;
        while pos < len && attempts < len {
            if let Some(line) = self.mgmt_lines.get(pos) {
                if Self::is_mgmt_selectable(line) {
                    break;
                }
            } else {
                break;
            }
            if forward {
                if pos + 1 < len {
                    pos += 1;
                } else {
                    break;
                }
            } else if pos > 0 {
                pos -= 1;
            } else {
                break;
            }
            attempts += 1;
        }
        self.cursor_right = pos;
    }

    pub fn clamp_now(&mut self) {
        self.clamp_cursor()
    }

    /// Dynamic number of rows currently in the right management panel based on
    /// the currently selected service and detail state.
    pub fn right_panel_len(&self) -> usize {
        if self.detail.is_none() {
            return self.config_rows.len().max(1);
        }
        if let Some(svc) = self.services.get(self.cursor_left) {
            if [
                fine_ill_do_it_myself::orchestrator::CADDY,
                fine_ill_do_it_myself::orchestrator::APACHE,
                fine_ill_do_it_myself::orchestrator::NGINX,
                fine_ill_do_it_myself::orchestrator::LIGHTTPD,
            ]
            .contains(&svc.name.as_str())
            {
                // Use the cached line count so the cursor can reach
                // all rows including dynamically discovered versions.
                return self.mgmt_line_count.max(4);
            }
            if svc.name == "mariadb" {
                return 4;
            }
            if svc.name.starts_with("php-") {
                let n = self
                    .config_rows
                    .iter()
                    .filter(|r| matches!(r, ConfigRow::UsePhp(_)))
                    .count();
                return n.max(1);
            }
        }
        self.config_rows.len().max(1)
    }

    fn clamp_cursor(&mut self) {
        let len = if self.panel_right {
            self.right_panel_len()
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

/// Which panel is focused given a cursor that walks services, languages, then projects.
pub fn focused_panel(app: &App) -> &'static str {
    if app.panel_right {
        "management"
    } else {
        let non_lang_count = app
            .services
            .iter()
            .filter(|s| !is_language(&s.name))
            .count();
        let lang_count = app.services.iter().filter(|s| is_language(&s.name)).count();
        let cur = app.cursor();
        if cur < non_lang_count {
            "services"
        } else if cur < non_lang_count + lang_count {
            "languages"
        } else if cur < non_lang_count + lang_count + app.projects.len() {
            "projects"
        } else {
            "services"
        }
    }
}

/// Port label per service row, shown beside its name.
pub fn ports_label(name: &str, php_row: &str, ports: &fine_ill_do_it_myself::PortPlan) -> String {
    if name == "mariadb" {
        ports.mariadb.to_string()
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

/// Management lines for ONE web server row: its default-selection action,
/// probed version, and its OWN http/https pair — never a shared list.
pub fn web_mgmt_lines(
    name: &str,
    cur_web: &str,
    ver: Option<&str>,
    ports: &fine_ill_do_it_myself::PortPlan,
) -> Vec<String> {
    let (http, https) = ports.web_ports(name);
    let ver_txt = ver.unwrap_or("not installed");
    let mut out = Vec::new();
    if name == cur_web {
        out.push("  default web (this one)".to_string());
    } else {
        out.push(format!("web.use {name}"));
    }
    out.push(format!("  version {ver_txt}"));
    out.push(format!("  port http = {http}"));
    out.push(format!("  port https = {https}"));
    out
}

/// Footer hint list as (key text, description) pairs, context aware: while
/// the port editor is open only its keys are shown. Duplicate key texts
/// (e.g. a remapped keybind colliding with a fixed key) are dropped,
/// keeping the first occurrence.
pub fn footer_hints(app: &App) -> Vec<(String, &'static str)> {
    use crossterm::event::KeyCode;
    let disp = |code: KeyCode| match code {
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    };
    if app.edit.is_some() {
        return vec![
            ("0-9".to_string(), "port digit"),
            ("backspace".to_string(), "hapus"),
            ("enter".to_string(), "terapkan"),
            ("esc".to_string(), "batal"),
        ];
    }
    let fixed = [
        (disp(app.keybinds.quit), "quit"),
        ("j".to_string(), "down"),
        ("k".to_string(), "up"),
        ("d".to_string(), "detail"),
        ("enter".to_string(), "apply"),
        ("tab".to_string(), "panel"),
        (disp(app.keybinds.start), "start"),
        (disp(app.keybinds.stop), "stop"),
        (disp(app.keybinds.restart), "restart"),
        (disp(app.keybinds.restart_all), "all"),
    ];
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for (k, l) in fixed {
        if !out.iter().any(|(seen, _)| seen == &k) {
            out.push((k, l));
        }
    }
    out
}

/// Discovered alternate versions of a web server in `bin/` or `bin/<server>/` (e.g. caddy-2.9.1).
pub fn installed_server_versions(env: &fine_ill_do_it_myself::Env, server: &str) -> Vec<String> {
    let mut vers = Vec::new();
    let prefix = format!("{server}-");
    // 1. Check in server-specific subdirectory bin/<server>/
    let server_dir = env.paths.bin().join(server);
    if let Ok(rd) = std::fs::read_dir(&server_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(v) = n.strip_prefix(&prefix) {
                if !v.ends_with(".old")
                    && !v.ends_with("-bin")
                    && !v.starts_with('.')
                    && !vers.contains(&v.to_string())
                {
                    vers.push(v.to_string());
                }
            } else if n != server
                && !n.ends_with(".old")
                && !n.ends_with("-bin")
                && !n.starts_with('.')
                && n.chars().next().is_some_and(|c| c.is_ascii_digit())
                && !vers.contains(&n)
            {
                vers.push(n);
            }
        }
    }
    // 2. Also check flat bin/ for backward compatibility
    if let Ok(rd) = std::fs::read_dir(env.paths.bin()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(v) = n.strip_prefix(&prefix) {
                if !v.ends_with(".old")
                    && !v.ends_with("-bin")
                    && !v.starts_with('.')
                    && !vers.contains(&v.to_string())
                {
                    vers.push(v.to_string());
                }
            }
        }
    }
    vers.sort_by(|a, b| {
        let parse = |s: &str| {
            s.split('.')
                .map(|p| p.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        };
        parse(b).cmp(&parse(a))
    });
    vers
}

/// Title + exact text of the management panel's items for `app`'s current
/// state. Shared by the renderer and by `App::mgmt_enter` so Enter always
/// acts on precisely the line that is displayed.
pub fn mgmt_panel(env: &fine_ill_do_it_myself::Env, app: &App) -> (String, Vec<String>) {
    let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
    let cur_php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
    let cur_web = fine_ill_do_it_myself::orchestrator::web_server(env);
    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());

    if let Some(svc) = app.services.get(app.cursor_left) {
        if svc.name.starts_with("php-") || svc.name == "php" {
            return (
                format!(" php versions (current: {cur_php}) "),
                fine_ill_do_it_myself::orchestrator::installed_php_versions(env)
                    .into_iter()
                    .map(|v| {
                        let mark = if v == cur_php { "▶ " } else { "  " };
                        format!("{mark}php.use {v}")
                    })
                    .collect(),
            );
        }
        for lang in ["node", "python", "go", "java"] {
            if svc.name == lang || svc.name.starts_with(&format!("{lang}-")) {
                let cur = fine_ill_do_it_myself::lang::active_lang_version(env, lang)
                    .unwrap_or_else(|| "none".into());
                let versions = fine_ill_do_it_myself::lang::installed_lang_versions(env, lang);
                let mut lines = Vec::new();
                if versions.is_empty() {
                    lines.push("  no versions installed:".to_string());
                    lines.push(format!("  install with: fidim {lang} install <v>:"));
                } else {
                    for v in &versions {
                        let is_active = v == &cur
                            || (v == "system" && (cur == "system" || cur.starts_with("system")));
                        let mark = if is_active { "▶ " } else { "  " };
                        let label = if v == "system" {
                            if let Some(sys) = fine_ill_do_it_myself::lang::which_system_lang(lang)
                            {
                                if let Some(sys_ver) =
                                    fine_ill_do_it_myself::lang::probe_version(&sys)
                                {
                                    format!("system ({sys_ver})")
                                } else {
                                    "system".to_string()
                                }
                            } else {
                                "system".to_string()
                            }
                        } else {
                            v.clone()
                        };
                        lines.push(format!("{mark}{lang}.use {label}"));
                    }
                }
                return (format!(" {lang} versions (current: {cur}) "), lines);
            }
        }
        if [
            fine_ill_do_it_myself::orchestrator::CADDY,
            fine_ill_do_it_myself::orchestrator::APACHE,
            fine_ill_do_it_myself::orchestrator::NGINX,
            fine_ill_do_it_myself::orchestrator::LIGHTTPD,
        ]
        .contains(&svc.name.as_str())
        {
            let ver = fine_ill_do_it_myself::orchestrator::web_version(env, &svc.name);
            let mut lines = web_mgmt_lines(&svc.name, &cur_web, ver.as_deref(), &ports);
            let vers = installed_server_versions(env, &svc.name);
            if !vers.is_empty() {
                lines.push(String::new());
                lines.push(format!("  {} versions:", svc.name));
                let cur_v = ver.unwrap_or_default();
                for v in vers {
                    let mark = if cur_v.starts_with(&v) || v.starts_with(&cur_v) {
                        "▶ "
                    } else {
                        "  "
                    };
                    lines.push(format!("{mark}{}.use {v}", svc.name));
                }
            }
            return (format!(" {} management ", svc.name), lines);
        }
        if svc.name == "mariadb" {
            return (
                " ports ".to_string(),
                vec![
                    format!("  port http = {}", ports.http),
                    format!("  port https = {}", ports.https),
                    format!("  port fpm_base = {}", ports.fpm_base),
                    format!("  port mariadb = {}", ports.mariadb),
                ],
            );
        }
    }
    // no service focused: the global config list
    (
        " config (select service) ".to_string(),
        app.config_rows
            .iter()
            .map(|r| match r {
                ConfigRow::UsePhp(v) => {
                    let mark = if *v == cur_php { "▶ " } else { "  " };
                    format!("{mark}php.use {v}")
                }
                ConfigRow::UseWeb { name, ver } => {
                    let mark = if name == &cur_web { "▶ " } else { "  " };
                    let v = ver.as_deref().unwrap_or("not installed");
                    format!("{mark}web.use {name}  [{v}]")
                }
                ConfigRow::Port { key, value } => format!("  port {key} = {value}"),
            })
            .collect(),
    )
}

/// Terminal entry: raw mode loop with ratatui rendering of status + logs.
pub fn run_tui(
    env: &fine_ill_do_it_myself::Env,
    _ports: fine_ill_do_it_myself::PortPlan,
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
    let act = make_actioner(env.clone());
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
        let ports =
            fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
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
                        Constraint::Percentage(65),
                        Constraint::Percentage(35),
                    ])
                    .areas(area);
                    (left, Some(right))
                } else {
                    (area, None)
                };

                let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
                let php_row = fine_ill_do_it_myself::orchestrator::default_php(&lock);

                let non_lang_indices: Vec<usize> = app
                    .services
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !is_language(&s.name))
                    .map(|(i, _)| i)
                    .collect();

                // Collapse language versions: show only one row per language
                // family (php, node, python, go, java). `language_reps` holds
                // the active version row for each family and is what navigation
                // walks, so the highlight never lands on a hidden version.
                let collapsed_lang: Vec<(usize, String)> = app
                    .language_reps
                    .iter()
                    .filter_map(|&idx| {
                        app.services
                            .get(idx)
                            .map(|s| (idx, language_family(&s.name).to_string()))
                    })
                    .collect();

                let svc_height = if non_lang_indices.is_empty() {
                    0
                } else {
                    non_lang_indices.len() as u16 + 2
                };

                let lang_height = if collapsed_lang.is_empty() {
                    0
                } else {
                    collapsed_lang.len() as u16 + 2
                };

                let [services, languages, projects, footer] = Layout::vertical([
                    Constraint::Length(svc_height),
                    Constraint::Length(lang_height),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .areas(main_area);

                let left_cursor = app.cursor_left;
                let svc_focus = !app.panel_right && non_lang_indices.contains(&left_cursor);
                let lang_focus = !app.panel_right && app.language_reps.contains(&left_cursor);
                let proj_focus = !app.panel_right && left_cursor >= app.services.len();

                // 1. Services panel (web servers, databases, etc.)
                if svc_height > 0 {
                    let items: Vec<ListItem> = non_lang_indices
                        .iter()
                        .map(|&idx| {
                            let s = &app.services[idx];
                            let (mark, name, color) = match &s.state {
                                ServiceState::Running => {
                                    ("● ".to_string(), s.name.clone(), Color::Green)
                                }
                                ServiceState::Error(msg) => {
                                    ("✗ ".to_string(), format!("{} ({msg})", s.name), Color::Red)
                                }
                                _ => ("○ ".to_string(), s.name.clone(), Color::DarkGray),
                            };
                            let p = ports_label(&s.name, &php_row, &ports);
                            let (p1, p2) = if let Some((http, https)) = p.split_once(' ') {
                                (http.to_string(), https.to_string())
                            } else if s.name == "mariadb" && !p.is_empty() {
                                (format!("port:{p}"), String::new())
                            } else {
                                (p, String::new())
                            };
                            let mut spans = vec![
                                Span::styled(mark, Style::default().fg(color)),
                                Span::styled(
                                    format!("{:<11}", name),
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ];
                            if !p1.is_empty() {
                                spans.push(Span::styled(
                                    format!("{:<12}", p1),
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                            if !p2.is_empty() {
                                spans.push(Span::styled(p2, Style::default().fg(Color::DarkGray)));
                            }
                            ListItem::new(Line::from(spans))
                        })
                        .collect();

                    let mut lst = ListState::default();
                    if let Some(pos) = non_lang_indices.iter().position(|&i| i == left_cursor) {
                        lst.select(Some(pos));
                    }
                    let svc_marker = if svc_focus { "▶ " } else { "  " };
                    let svc_list = List::new(items)
                        .highlight_symbol(svc_marker.to_owned())
                        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
                        .block(
                            Block::default()
                                .title(format!(" services{} ", if svc_focus { " ◀" } else { "" }))
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(if svc_focus {
                                    Color::Cyan
                                } else {
                                    Color::DarkGray
                                })),
                        );
                    f.render_stateful_widget(svc_list, services, &mut lst);
                }

                // 2. Programming languages panel (PHP runtimes, etc.)
                if lang_height > 0 {
                    let lang_items: Vec<ListItem> = collapsed_lang
                        .iter()
                        .map(|(idx, display_name)| {
                            let s = &app.services[*idx];
                            let p = if display_name == "php" {
                                ports_label(&s.name, &php_row, &ports)
                            } else {
                                fine_ill_do_it_myself::lang::active_lang_version(env, display_name)
                                    .unwrap_or_else(|| "not installed".to_string())
                            };
                            let mut spans = vec![Span::styled(
                                format!("{:<13}", display_name),
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            )];
                            if !p.is_empty() {
                                spans.push(Span::styled(
                                    format!("{:<12}", p),
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                            ListItem::new(Line::from(spans))
                        })
                        .collect();

                    let mut lang_lst = ListState::default();
                    if let Some(pos) = collapsed_lang.iter().position(|(i, _)| *i == left_cursor) {
                        lang_lst.select(Some(pos));
                    }
                    let lang_marker = if lang_focus { "▶ " } else { "  " };
                    let lang_list = List::new(lang_items)
                        .highlight_symbol(lang_marker.to_owned())
                        .highlight_style(Style::default().bg(Color::Green).fg(Color::Black))
                        .block(
                            Block::default()
                                .title(format!(
                                    " programming languages (php: {php_row}){} ",
                                    if lang_focus { " ◀" } else { "" }
                                ))
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(if lang_focus {
                                    Color::Green
                                } else {
                                    Color::DarkGray
                                })),
                        );
                    f.render_stateful_widget(lang_list, languages, &mut lang_lst);
                }

                // 3. Projects panel
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
                        left_cursor
                            .saturating_sub(app.services.len())
                            .min(app.projects.len().saturating_sub(1)),
                    ));
                }
                let proj_list = List::new(proj_items)
                    .highlight_symbol(if proj_focus { "▶ " } else { "  " }.to_owned())
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
                f.render_stateful_widget(proj_list, projects, &mut plst); // detail panel - shown when 'd' is pressed; items come from
                                                                          // mgmt_panel so Enter (mgmt_enter) acts on exactly these lines
                if let Some(detail_area) = detail_area {
                    let (mgmt_title, lines) = mgmt_panel(env, &app);
                    app.mgmt_line_count = lines.len();
                    app.mgmt_lines = lines.clone();
                    let mgmt_focus = app.panel_right;
                    let mgmt_items: Vec<ListItem> = lines
                        .iter()
                        .map(|line| {
                            let selectable = App::is_mgmt_selectable(line);
                            let color = if !selectable {
                                Color::DarkGray
                            } else if line.starts_with("web.use") {
                                Color::Cyan
                            } else {
                                Color::Yellow
                            };
                            ListItem::new(Line::from(Span::styled(
                                line.clone(),
                                Style::default().fg(color),
                            )))
                        })
                        .collect();

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

                // footer with key hints (caller-owned list, context aware)
                let hints = footer_hints(&app);
                let mut spans: Vec<Span> = Vec::new();
                for (key, label) in hints {
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
                    // Enter on the right panel acts on the highlighted
                    // management line (port editor, web.use, php.use) ONLY
                    // when the editor is not already open.  If the editor is
                    // active the keystroke must reach on_key so it can commit
                    // the typed value via SetPort (the bug: without this guard
                    // mgmt_enter would re-open the editor with the stale
                    // prefill and silently discard the user's input).
                    if k.code == crossterm::event::KeyCode::Enter
                        && app.panel_right
                        && app.detail.is_some()
                        && app.edit.is_none()
                    {
                        let (_, lines) = mgmt_panel(env, &app);
                        app.mgmt_line_count = lines.len();
                        app.mgmt_lines = lines.clone();
                        app.mgmt_enter(&lines);
                    } else {
                        app.on_key(k.code);
                    }
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
        let mut statuses = fine_ill_do_it_myself::orchestrator::status_all(
            &env,
            &ports,
            &fine_ill_do_it_myself::RealRunner,
        );
        statuses.sort_by_key(|s| is_language(&s.name));
        app.services = statuses;
        app.language_reps = language_representatives(&env, &app.services);

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
        app.normalize_left_cursor();
    }
}

/// Execute one queued action against the system; sets the status-bar
/// message with the outcome. Ports are re-resolved from `etc/ports.toml` on
/// every execution so a SetPort followed by any start/restart picks up the
/// saved values instead of a snapshot from TUI startup.
fn perform_action<R: fine_ill_do_it_myself::Runner + Clone>(
    env: &fine_ill_do_it_myself::Env,
    runner: R,
    app: &mut App,
    action: &Action,
) {
    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
    let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
    let php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
    let r: Result<Vec<_>, _> = match action {
        Action::RestartAll => fine_ill_do_it_myself::orchestrator::stop_all(env, &ports, &runner)
            .and_then(|_| {
                fine_ill_do_it_myself::orchestrator::start_all(env, &ports, &runner, &php)
            }),
        Action::StartService(i) => {
            let name = app.services[*i].name.clone();
            fine_ill_do_it_myself::orchestrator::start_service(env, &ports, &runner, &php, &name)
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
                    env, &ports, &runner, &php, &name,
                )
                .map(|s| vec![s])
            })
        }
        Action::UsePhp(v) => {
            let cli_env = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: v.clone(),
            };
            crate::cli::php_use(&cli_env, v).map(|_| vec![])
        }
        Action::UseLang { lang, version } => {
            let cli_env = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: version.clone(),
            };
            crate::cli::lang_use(&cli_env, lang, version).map(|_| vec![])
        }
        Action::UseWeb(name) => {
            if fine_ill_do_it_myself::orchestrator::resolve_web_bin(env, name).is_err() {
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
                        fine_ill_do_it_myself::orchestrator::generate_configs(env, &ports, &php)
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
            // shared with the CLI: accepts the per-server pairs too
            // (nginx_http, ...) and rejects unknown keys with an error
            crate::cli::port_set(&env.paths.etc(), key, *value).map(|_| vec![])
        }
        Action::UseServerVersion { server, version } => {
            let server_dir = env.paths.bin().join(server);
            let (src, dest) = if server_dir.is_dir() {
                let candidate1 = server_dir.join(format!("{server}-{version}"));
                let candidate2 = server_dir.join(version);
                let src = if candidate1.exists() {
                    candidate1
                } else if candidate2.exists() {
                    candidate2
                } else {
                    candidate1
                };
                let dest = server_dir.join(server);
                (src, dest)
            } else {
                let src = env.paths.bin().join(format!("{server}-{version}"));
                let dest = env.paths.bin().join(server);
                (src, dest)
            };
            if !src.exists() {
                Err(fine_ill_do_it_myself::Error::Other(format!(
                    "version {version} for {server} not found"
                )))
            } else {
                let _ = std::fs::remove_file(&dest);
                #[cfg(unix)]
                let res = {
                    use std::os::unix::fs::symlink;
                    symlink(&src, &dest).or_else(|_| std::fs::copy(&src, &dest).map(|_| ()))
                };
                #[cfg(not(unix))]
                let res = std::fs::copy(&src, &dest).map(|_| ());
                res.map(|_| vec![])
                    .map_err(fine_ill_do_it_myself::Error::Io)
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
                    if let Some(bin) = fine_ill_do_it_myself::orchestrator::php_binary_for(env, ver)
                    {
                        detail.push_str(&format!("Binary: {}\n", bin.display()));
                    }
                }
            } else if ["caddy", "httpd", "nginx", "lighttpd"].contains(&name.as_str()) {
                detail.push_str("Web Server\n");
                if let Some(ver) = fine_ill_do_it_myself::orchestrator::web_version(env, name) {
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
        Action::OpenProject(project) => {
            let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
            let web = fine_ill_do_it_myself::orchestrator::default_web(&lock);
            let (http_port, _) = ports.web_ports(&web);
            let url = if http_port == 80 {
                format!("http://{project}.test")
            } else {
                format!("http://{project}.test:{http_port}")
            };
            let _ = std::process::Command::new("xdg-open")
                .arg(&url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            let hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
            if !hosts.contains(&format!("{project}.test")) {
                Err(fine_ill_do_it_myself::Error::Other(format!(
                    "opened {url} (note: {project}.test missing in /etc/hosts, run: fidim hosts --sync)"
                )))
            } else {
                Ok(vec![])
            }
        }
    };
    match r {
        Ok(_) => match action {
            Action::OpenProject(p) => {
                let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
                let web = fine_ill_do_it_myself::orchestrator::default_web(&lock);
                let (http_port, _) = ports.web_ports(&web);
                let port_str = if http_port == 80 {
                    String::new()
                } else {
                    format!(":{http_port}")
                };
                app.message = format!("opened http://{p}.test{port_str} in browser");
            }
            _ => app.message = format!("{action:?} ok").to_lowercase(),
        },
        Err(e) => app.message = format!("{action:?}: {e}").to_lowercase(),
    }
}

/// Build the real-system action handler for the TUI event loop.
fn make_actioner(env: fine_ill_do_it_myself::Env) -> impl Fn(&mut App, &Action) {
    move |app: &mut App, action: &Action| {
        perform_action(&env.clone(), fine_ill_do_it_myself::RealRunner, app, action);
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
    fn arrows_clamp_inside_the_services_container() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("mariadb", false)],
            vec!["blog".into()],
        );
        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor_left, 1);
        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor_left, 1, "must not walk into projects");
        assert_eq!(focused_panel(&a), "services");
        a.on_key(KeyCode::Up);
        assert_eq!(a.cursor_left, 0);
        a.on_key(KeyCode::Up);
        assert_eq!(a.cursor_left, 0, "clamped at the first service");
    }

    #[test]
    fn arrows_clamp_inside_the_projects_container() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["a".into(), "b".into()]);
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "projects", "no languages installed");

        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor_left, 2);
        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor_left, 2, "clamped at the last project");
    }

    #[test]
    fn tab_cycles_through_every_container() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("php-8.4", false)],
            vec!["blog".into()],
        );
        assert_eq!(focused_panel(&a), "services");

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "languages");
        assert_eq!(a.cursor_left, 1);

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "projects");
        assert_eq!(a.cursor_left, 2);

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "management");

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "services");
        assert_eq!(a.cursor_left, 0);
    }

    #[test]
    fn tab_skips_containers_without_rows() {
        let mut a = App::new(vec![svc("caddy", false)], vec![]);

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "management");

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "services");
    }

    #[test]
    fn backtab_walks_containers_backwards() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("php-8.4", false)],
            vec!["blog".into()],
        );
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "languages");

        a.on_key(KeyCode::BackTab);
        assert_eq!(focused_panel(&a), "services");

        a.on_key(KeyCode::BackTab);
        assert_eq!(focused_panel(&a), "management");
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
        a.on_key(KeyCode::Tab); // services -> languages (php-8.4)
        a.on_key(KeyCode::Char('x'));
        assert_eq!(a.actions[0], Action::StopService(1));
        a.actions.clear();
        a.on_key(KeyCode::Char('r'));
        assert_eq!(a.actions[0], Action::RestartService(1));
        // on the projects container `r` restarts everything
        a.actions.clear();
        a.on_key(KeyCode::Tab); // languages -> projects
        a.on_key(KeyCode::Char('r'));
        assert_eq!(a.actions[0], Action::RestartAll);
    }

    #[test]
    fn language_panel_navigation_skips_hidden_versions() {
        let mut a = App::new(
            vec![
                svc("caddy", false),
                svc("mariadb", false),
                svc("php-8.1", false),
                svc("php-8.2", false),
                svc("node-20", false),
                svc("node-24", false),
                svc("python-3.13", false),
                svc("go-1.27", false),
                svc("java-25", false),
            ],
            vec!["blog".into()],
        );

        // Tab into the languages container; from there one step per visible
        // family row. Hidden versions must not consume key presses, and the
        // arrows must not leak into the neighbouring containers.
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "languages");
        assert_eq!(a.cursor_left, 2, "starts on the php family row");

        let mut down = vec![a.cursor_left];
        for _ in 0..4 {
            a.on_key(KeyCode::Down);
            down.push(a.cursor_left);
        }
        assert_eq!(down, vec![2, 4, 6, 7, 8], "one row per language family");

        let mut up = Vec::new();
        for _ in 0..5 {
            a.on_key(KeyCode::Up);
            up.push(a.cursor_left);
        }
        assert_eq!(up, vec![7, 6, 4, 2, 2], "up path clamps at the top");
    }

    #[test]
    fn language_representatives_follow_active_versions() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let mut language_defaults = std::collections::BTreeMap::new();
        language_defaults.insert("node".into(), "24".into());
        let lock = fine_ill_do_it_myself::Lock {
            php_default: Some("8.2".into()),
            language_defaults,
            ..Default::default()
        };
        lock.save(&env.lock_path()).unwrap();

        let services = vec![
            svc("php-8.1", false),
            svc("php-8.2", false),
            svc("node-20", false),
            svc("node-24", false),
        ];
        assert_eq!(
            language_representatives(&env, &services),
            vec![1, 3],
            "php 8.2 and node 24 are the active rows"
        );
    }

    #[test]
    fn hidden_language_cursor_normalizes_to_the_representative() {
        let mut a = App::new(vec![svc("php-8.1", false), svc("php-8.2", false)], vec![]);
        a.language_reps = vec![1];
        a.cursor_left = 0;
        a.normalize_left_cursor();
        assert_eq!(a.cursor_left, 1);
    }

    #[test]
    fn focused_panel_follows_cursor_into_projects() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        assert_eq!(focused_panel(&a), "services");
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "projects", "no languages installed");
        a.on_key(KeyCode::BackTab);
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
    fn set_port_action_persists_per_web_server_keys() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        let act = make_actioner(env.clone());
        let mut app = App::new(vec![], vec![]);
        act(
            &mut app,
            &Action::SetPort {
                key: "nginx_http".into(),
                value: 9090,
            },
        );
        let text = std::fs::read_to_string(env.paths.etc().join("ports.toml")).unwrap();
        assert!(text.contains("nginx_http = 9090"), "{text}");
    }

    #[test]
    fn set_port_action_rejects_unknown_keys() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        let act = make_actioner(env.clone());
        let mut app = App::new(vec![], vec![]);
        act(
            &mut app,
            &Action::SetPort {
                key: "banana".into(),
                value: 1,
            },
        );
        assert!(
            !std::fs::exists(env.paths.etc().join("ports.toml")).unwrap(),
            "unknown key must not create ports.toml"
        );
    }

    #[test]
    fn restart_after_set_port_picks_up_the_new_saved_ports() {
        // aka `main` of the bug: SetPort saves 8290 into ports.toml, then the
        // user restarts nginx from the TUI — the regenerated site must listen
        // on 8290, not the startup snapshot's default
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        std::fs::create_dir_all(env.paths.sites().join("blog")).unwrap();
        std::fs::create_dir_all(env.paths.bin()).unwrap();
        // scripts that exit immediately so RealRunner spawns are harmless
        for bin in ["caddy", "nginx", "php-fpm-8.4"] {
            let p = env.paths.bin().join(bin);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
        }
        let act = make_actioner(env.clone());
        let mut app = App::new(vec![svc("nginx", false)], vec![]);
        act(
            &mut app,
            &Action::SetPort {
                key: "nginx_http".into(),
                value: 8290,
            },
        );
        act(&mut app, &Action::RestartService(0));
        let site = std::fs::read_to_string(env.paths.etc().join("nginx/sites/blog.conf")).unwrap();
        assert!(
            site.contains("listen 8290;"),
            "restart after SetPort must use the saved port\n{site}"
        );
    }

    #[test]
    fn editor_typing_replaces_the_prefill() {
        let mut a = App::new(vec![svc("nginx", false)], vec![]);
        a.detail = Some("detail".into());
        a.panel_right = true;
        a.clamp_now();
        a.mgmt_enter(&["port http = 8082".to_string()]);
        assert_eq!(a.input, "8082", "prefilled with current value");
        // user types the replacement port; the prefill must not be appended to
        for c in ['9', '0', '9', '0'] {
            a.on_key(KeyCode::Char(c));
        }
        assert_eq!(a.input, "9090", "typing replaces the prefilled value");
        a.on_key(KeyCode::Enter);
        assert!(
            a.actions.contains(&Action::SetPort {
                key: "nginx_http".into(),
                value: 9090,
            }),
            "applied action must carry the typed port, got {:?}",
            a.actions
        );
    }

    #[test]
    fn footer_hints_list_every_key_in_default_view() {
        let a = App::new(vec![], vec![]);
        let h = footer_hints(&a);
        let keys: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        // fixed navigation & selection keys
        for want in ["d", "enter", "tab", "j", "k"] {
            assert!(keys.contains(&want), "missing key {want}\n{keys:?}");
        }
        // remapable keybinds (defaults q/r/s/x/a)
        for want in ["q", "s", "x", "r", "a"] {
            assert!(keys.contains(&want), "missing keybind {want}\n{keys:?}");
        }
        // no duplicated key chips
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            keys.len(),
            "duplicate key hints ({keys:?} vs {uniq:?})"
        );
    }

    #[test]
    fn footer_hints_switch_to_editor_keys_while_editing() {
        let mut a = App::new(vec![], vec![]);
        a.edit = Some(Edit::Port("nginx_http".into()));
        let h = footer_hints(&a);
        let keys: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        for want in ["0-9", "backspace", "enter", "esc"] {
            assert!(keys.contains(&want), "missing editor key {want}\n{keys:?}");
        }
        // the normal key set is hidden while typing a port
        assert!(!keys.contains(&"q"), "{keys:?}");
    }

    #[test]
    fn management_enter_opens_port_edit_with_server_specific_key() {
        let mut a = App::new(vec![svc("nginx", false)], vec![]);
        a.detail = Some("detail".into());
        a.panel_right = true;
        a.cursor_right = 2; // highlighted line: "port http = 8082"
        let lines = vec![
            "web.use nginx".to_string(),
            "  version 1.2".to_string(),
            "  port http = 8082".to_string(),
            "  port https = 8445".to_string(),
        ];
        a.mgmt_enter(&lines);
        assert_eq!(a.input, "8082", "edit input prefilled with current value");
        match &a.edit {
            Some(Edit::Port(k)) => {
                assert_eq!(k, "nginx_http", "must use the server-specific toml key")
            }
            other => panic!("expected port edit, got {other:?}"),
        }
        // apply: editor Enter parses input and queues SetPort
        a.on_key(KeyCode::Enter);
        assert!(a.actions.contains(&Action::SetPort {
            key: "nginx_http".into(),
            value: 8082
        }));
    }

    #[test]
    fn management_enter_dispatches_web_use_and_php_use() {
        let mut w = App::new(vec![svc("nginx", false)], vec![]);
        w.detail = Some("d".into());
        w.panel_right = true;
        w.mgmt_enter(&["web.use nginx".to_string()]);
        assert_eq!(w.actions.first(), Some(&Action::UseWeb("nginx".into())));

        let mut p = App::new(vec![svc("php-8.4", false)], vec![]);
        p.detail = Some("d".into());
        p.panel_right = true;
        p.cursor_right = 1;
        p.mgmt_enter(&["▶ php.use 8.4".to_string(), "  php.use 8.3".to_string()]);
        assert_eq!(p.actions.first(), Some(&Action::UsePhp("8.3".into())));
    }

    #[test]
    fn management_enter_maps_global_ports_outside_web_rows() {
        let mut a = App::new(vec![svc("mariadb", false)], vec![]);
        a.detail = Some("d".into());
        a.panel_right = true;
        a.cursor_right = 0; // "port http = 80"
        a.mgmt_enter(&[
            "  port http = 80".to_string(),
            "  port https = 443".to_string(),
        ]);
        match &a.edit {
            Some(Edit::Port(k)) => assert_eq!(k, "http", "mariadb row shows the global pair"),
            other => panic!("expected port edit, got {other:?}"),
        }
    }

    #[test]
    fn web_mgmt_is_per_server_not_one_shared_list() {
        let ports = fine_ill_do_it_myself::PortPlan::default();
        let nginx = web_mgmt_lines("nginx", "caddy", Some("1.29.1"), &ports);
        let caddy = web_mgmt_lines("caddy", "caddy", Some("2.10.0"), &ports);
        // nginx panel: its own action, version and ITS OWN ports
        assert!(
            nginx.iter().any(|l| l.contains("web.use nginx")),
            "{nginx:?}"
        );
        assert!(
            nginx.iter().any(|l| l.contains("version 1.29.1")),
            "{nginx:?}"
        );
        assert!(
            nginx.iter().any(|l| l.contains("port http = 8082")),
            "{nginx:?}"
        );
        assert!(
            nginx.iter().any(|l| l.contains("port https = 8445")),
            "{nginx:?}"
        );
        assert!(
            !nginx.iter().any(|l| l.contains("port http = 8080")),
            "{nginx:?}"
        );
        assert!(!nginx.iter().any(|l| l.contains("caddy")), "{nginx:?}");
        // caddy panel: marked as default, caddy's own ports
        assert!(caddy.iter().any(|l| l.contains("default web")), "{caddy:?}");
        assert!(
            caddy.iter().any(|l| l.contains("port http = 8080")),
            "{caddy:?}"
        );
        assert!(!caddy.iter().any(|l| l.contains("web.use")), "{caddy:?}");
        // panels differ per server, not one shared list
        assert_ne!(nginx, caddy);
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

    /// Regression: when the port editor is open and the user presses Enter,
    /// `run_tui`'s routing must hand Enter to `on_key` (which commits the
    /// typed value) — NOT to `mgmt_enter` (which would re-open/reset the
    /// editor with the stale value from the panel lines, causing every edit
    /// to be silently discarded and the port to revert to the default).
    ///
    /// The helper `dispatch_enter` mirrors the exact `if`-branch in `run_tui`:
    ///   • editor inactive + right panel + detail → mgmt_enter
    ///   • everything else (including editor active) → on_key
    #[test]
    fn enter_while_editor_open_commits_value_not_resets_editor() {
        // Simulates the run_tui dispatch logic:
        //   if Enter && panel_right && detail.is_some() && edit.is_none() → mgmt_enter
        //   else → on_key
        fn dispatch_enter(app: &mut App, lines: &[String]) {
            use crossterm::event::KeyCode;
            if app.panel_right && app.detail.is_some() && app.edit.is_none() {
                app.mgmt_enter(lines);
            } else {
                app.on_key(KeyCode::Enter);
            }
        }

        let mut a = App::new(vec![svc("nginx", false)], vec![]);
        a.detail = Some("detail".into());
        a.panel_right = true;
        a.cursor_right = 2; // "  port http = 8082"
        let lines = vec![
            "web.use nginx".to_string(),
            "  version 1.2".to_string(),
            "  port http = 8082".to_string(),
            "  port https = 8445".to_string(),
        ];

        // Step 1: first Enter opens the editor (editor is not yet active)
        dispatch_enter(&mut a, &lines);
        assert!(a.edit.is_some(), "first Enter must open the port editor");
        assert_eq!(a.input, "8082");

        // Step 2: user types a new port
        for c in ['9', '0', '9', '0'] {
            a.on_key(crossterm::event::KeyCode::Char(c));
        }
        assert_eq!(a.input, "9090");

        // Step 3: second Enter must COMMIT via on_key, NOT re-open via mgmt_enter.
        // Without the `app.edit.is_none()` guard, mgmt_enter would run here,
        // resetting `app.input` back to "8082" and discarding the SetPort action.
        dispatch_enter(&mut a, &lines);
        assert!(
            a.edit.is_none(),
            "editor must be closed after committing Enter"
        );
        assert!(
            a.actions.contains(&Action::SetPort {
                key: "nginx_http".into(),
                value: 9090,
            }),
            "SetPort action with the typed value must be queued; got {:?}",
            a.actions
        );
        // The old value must not appear in the action list
        assert!(
            !a.actions.contains(&Action::SetPort {
                key: "nginx_http".into(),
                value: 8082,
            }),
            "stale default value must not survive the edit"
        );
    }

    /// End-to-end: mirrors the exact key sequence a user performs in the TUI
    /// to change caddy's HTTP port from 8080 → 9090, including the perform_action
    /// call that writes the value to etc/ports.toml.
    ///
    /// Flow:
    ///   cursor on caddy → 'd' → right panel opens at line 0
    ///   → Down × 2 → line 2 = "  port http = 8080"
    ///   → Enter (edit.is_none()) → mgmt_enter → editor opens, input = "8080"
    ///   → '9','0','9','0' → input = "9090"
    ///   → Enter (edit.is_some()) → on_key → SetPort queued
    ///   → perform_action writes etc/ports.toml
    ///   → PortPlan::resolve reads the new file → caddy_http = 9090
    #[test]
    fn end_to_end_port_edit_writes_to_disk_and_survives_resolve() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let mut a = App::new(
            vec![
                svc("caddy", false),
                svc("mariadb", false),
                svc("php-8.4", false),
            ],
            vec![],
        );
        a.config_rows = vec![
            ConfigRow::UsePhp("8.4".into()),
            ConfigRow::UseWeb {
                name: "caddy".into(),
                ver: None,
            },
            ConfigRow::Port {
                key: "http".into(),
                value: 8080,
            },
            ConfigRow::Port {
                key: "https".into(),
                value: 8443,
            },
            ConfigRow::Port {
                key: "fpm_base".into(),
                value: 9000,
            },
            ConfigRow::Port {
                key: "mariadb".into(),
                value: 3306,
            },
        ];

        // 'd' on caddy → detail panel, cursor_right resets to 0
        a.on_key(KeyCode::Char('d'));
        assert!(a.panel_right);
        assert!(a.detail.is_some());
        assert_eq!(a.cursor_right, 0);

        // mgmt lines as caddy web_mgmt_lines returns (default ports)
        let caddy_lines = vec![
            "  default web (this one)".to_string(), // 0
            "  version not installed".to_string(),  // 1
            "  port http = 8080".to_string(),       // 2
            "  port https = 8443".to_string(),      // 3
        ];

        // dispatch_enter mirrors the fixed run_tui logic
        fn dispatch_enter(app: &mut App, lines: &[String]) {
            if app.panel_right && app.detail.is_some() && app.edit.is_none() {
                app.mgmt_enter(lines);
            } else {
                app.on_key(KeyCode::Enter);
            }
        }

        // Down × 2 → "  port http = 8080" (index 2)
        a.on_key(KeyCode::Down);
        a.on_key(KeyCode::Down);
        assert_eq!(a.cursor_right, 2);

        // Enter 1 → editor opens
        dispatch_enter(&mut a, &caddy_lines);
        assert!(a.edit.is_some(), "editor must open; edit={:?}", a.edit);
        assert_eq!(a.input, "8080");
        match &a.edit {
            Some(Edit::Port(k)) => assert_eq!(k, "caddy_http", "key must be caddy_http"),
            other => panic!("expected Edit::Port, got {other:?}"),
        }

        // type new port
        for c in ['9', '0', '9', '0'] {
            a.on_key(KeyCode::Char(c));
        }
        assert_eq!(a.input, "9090");

        // Enter 2 → commit
        dispatch_enter(&mut a, &caddy_lines);
        assert!(a.edit.is_none(), "editor must close after commit");
        assert!(
            a.actions.contains(&Action::SetPort {
                key: "caddy_http".into(),
                value: 9090
            }),
            "SetPort must be queued; got {:?}",
            a.actions
        );

        // perform_action writes to disk
        let act = make_actioner(env.clone());
        while let Some(action) = a.actions.pop() {
            act(&mut a, &action);
        }

        // verify file
        let text = std::fs::read_to_string(env.paths.etc().join("ports.toml")).unwrap_or_default();
        assert!(
            text.contains("caddy_http = 9090"),
            "ports.toml must have new value\n{text}"
        );
        assert!(
            !text.contains("caddy_http = 8080"),
            "old value must not remain\n{text}"
        );

        // verify PortPlan::resolve reads it back
        let resolved = fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |_| None);
        assert_eq!(resolved.caddy_http, 9090, "resolve must return saved value");
    }

    #[test]
    fn enter_on_a_language_runtime_opens_versions_instead_of_toggling_service() {
        let mut a = App::new(vec![svc("caddy", true), svc("node-24", true)], vec![]);
        a.cursor_left = 1;
        assert_eq!(focused_panel(&a), "languages");

        a.on_key(KeyCode::Enter);

        assert!(
            a.actions.is_empty(),
            "runtime rows must not start/stop anything: {:?}",
            a.actions
        );
        assert!(a.panel_right, "Enter should focus the version panel");
        assert!(a.detail.is_some(), "the version panel must be visible");
    }

    #[test]
    fn start_stop_restart_keys_ignore_language_runtimes() {
        let mut a = App::new(vec![svc("caddy", true), svc("python-3.13", true)], vec![]);
        a.cursor_left = 1;

        a.on_key(KeyCode::Char('s'));
        a.on_key(KeyCode::Char('x'));
        a.on_key(KeyCode::Char('r'));

        assert!(
            a.actions.is_empty(),
            "runtime rows are not services: {:?}",
            a.actions
        );
    }

    #[test]
    fn enter_on_php_fpm_row_still_toggles_the_service() {
        let mut a = App::new(vec![svc("php-8.4", true)], vec![]);
        assert_eq!(focused_panel(&a), "languages");

        a.on_key(KeyCode::Enter);

        assert_eq!(a.actions, vec![Action::StopService(0)]);
        assert!(!a.panel_right);
    }

    #[test]
    fn enter_on_a_service_row_still_toggles_the_service() {
        let mut a = App::new(vec![svc("caddy", false)], vec![]);
        a.on_key(KeyCode::Enter);

        assert_eq!(a.actions, vec![Action::StartService(0)]);
    }

    #[test]
    fn focused_panel_and_separation_for_languages() {
        let mut a = App::new(
            vec![
                svc("caddy", true),
                svc("php-8.4", false),
                svc("mariadb", true),
            ],
            vec!["blog".into()],
        );
        // Non-languages (caddy, mariadb) first:
        assert_eq!(a.services[0].name, "caddy");
        assert_eq!(a.services[1].name, "mariadb");
        // Programming languages (php-8.4) second:
        assert_eq!(a.services[2].name, "php-8.4");

        // Cursor at 0: services container
        assert_eq!(focused_panel(&a), "services");
        // Cursor at 1: still services
        a.on_key(KeyCode::Down);
        assert_eq!(focused_panel(&a), "services");
        // Tab crosses into the languages container
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "languages");
        // Tab again reaches projects
        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "projects");
    }

    #[test]
    fn server_versions_discovery_and_mgmt_enter() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        std::fs::create_dir_all(env.paths.bin()).unwrap();

        std::fs::write(env.paths.bin().join("caddy-2.10.2"), "").unwrap();
        std::fs::write(env.paths.bin().join("caddy-2.9.1"), "").unwrap();

        let vers = installed_server_versions(&env, "caddy");
        assert_eq!(vers, vec!["2.10.2", "2.9.1"]);

        let mut a = App::new(vec![svc("caddy", true)], vec![]);
        a.mgmt_enter(&["  caddy.use 2.9.1".to_string()]);
        assert_eq!(
            a.actions,
            vec![Action::UseServerVersion {
                server: "caddy".into(),
                version: "2.9.1".into()
            }]
        );
    }

    #[test]
    fn nested_server_versions_discovery_and_mgmt_enter() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        let caddy_dir = env.paths.bin().join("caddy");
        std::fs::create_dir_all(&caddy_dir).unwrap();

        std::fs::write(caddy_dir.join("caddy-2.10.2"), "").unwrap();
        std::fs::write(caddy_dir.join("caddy-2.9.1"), "").unwrap();

        let vers = installed_server_versions(&env, "caddy");
        assert_eq!(vers, vec!["2.10.2", "2.9.1"]);

        let mut a = App::new(vec![svc("caddy", true)], vec![]);
        a.mgmt_enter(&["  caddy.use 2.9.1".to_string()]);
        assert_eq!(
            a.actions,
            vec![Action::UseServerVersion {
                server: "caddy".into(),
                version: "2.9.1".into()
            }]
        );
    }
}
