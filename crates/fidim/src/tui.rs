use std::time::Duration;

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
    /// Install a web server from the bundled manifest.
    InstallWeb(String),
    /// Switch a programming language version (e.g. node -> node-22).
    UseLang { lang: String, version: String },
    /// Open a project in the default browser.
    OpenProject(String),
    /// Open a project folder in the user's code editor.
    OpenEditor(String),
    /// Download/refresh the managed composer.phar.
    InstallComposer,
    /// Install a PHP runtime and activate it.
    InstallPhp(String),
    /// Install a language runtime and activate it.
    InstallLang { lang: String, version: String },
    /// Install an optional managed component such as MariaDB or phpMyAdmin.
    InstallManaged(String),
    /// Create a new project from the TUI quick-create flow.
    CreateProject {
        name: String,
        template: fine_ill_do_it_myself::project::Template,
        db: bool,
    },
}

/// Focused field of the new-project modal form (`n` opens it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuickCreateField {
    /// Project name, typed in place.
    Name,
    /// Template list, walked with ↑/↓ or j/k.
    Template,
    /// MariaDB checkbox, toggled with Space.
    Database,
}

impl QuickCreateField {
    /// Next field on `Tab` (wraps back to the name).
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Template,
            Self::Template => Self::Database,
            Self::Database => Self::Name,
        }
    }

    /// Previous field on `Shift+Tab` (wraps to the database).
    fn prev(self) -> Self {
        match self {
            Self::Name => Self::Database,
            Self::Template => Self::Name,
            Self::Database => Self::Template,
        }
    }
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
    /// open the project folder in the code editor (o)
    pub editor: crossterm::event::KeyCode,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            quit: crossterm::event::KeyCode::Char('q'),
            restart: crossterm::event::KeyCode::Char('r'),
            start: crossterm::event::KeyCode::Char('s'),
            stop: crossterm::event::KeyCode::Char('x'),
            restart_all: crossterm::event::KeyCode::Char('a'),
            editor: crossterm::event::KeyCode::Char('o'),
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
                "editor" => k.editor = code,
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
    /// Editor resolved once at startup (`o` on a project row).
    pub editor: Option<fine_ill_do_it_myself::editor::Editor>,
    /// Terminal editors cannot run while the TUI owns the screen, so the
    /// action parks the argv here and the event loop hands the screen over.
    pub pending_terminal_editor: Option<Vec<String>>,
    /// New-project modal state. `None` = closed; `Some(field)` = open with
    /// that field focused.
    pub quick_create: Option<QuickCreateField>,
    /// Project name being typed in the modal.
    pub name_input: String,
    /// Index into `Template::ALL` on the template field.
    pub template_cursor: usize,
    /// Whether to also create a MariaDB database (`--db`).
    pub with_db: bool,
    /// Inline validation message shown inside the modal.
    pub form_error: Option<String>,
    /// Action currently running on the worker thread (`None` = idle).
    pub job: Option<Job>,
    /// Ports resolved during refresh; `render` never resolves them itself.
    pub ports: fine_ill_do_it_myself::PortPlan,
    /// Default PHP version, cached for the running-row label.
    pub php_row: String,
    /// Active version per language family, cached during refresh.
    pub lang_versions: std::collections::BTreeMap<String, String>,
    /// True while the background service/config refresh is running.
    refreshing: bool,
    /// Management panel snapshot currently being computed in the background.
    mgmt_pending: Option<(usize, bool, u64)>,
    /// Cached management panel title and lines (built by `refresh_mgmt`).
    pub mgmt_title: String,
    /// Inputs the cached management lines were built from.
    pub mgmt_for: Option<(usize, bool, u64)>,
    /// Bumped on every refresh so caches know they are stale.
    pub data_revision: u64,
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
            editor: None,
            pending_terminal_editor: None,
            quick_create: None,
            name_input: String::new(),
            template_cursor: default_template_index(),
            with_db: false,
            form_error: None,
            job: None,
            ports: fine_ill_do_it_myself::PortPlan::default(),
            php_row: String::new(),
            lang_versions: std::collections::BTreeMap::new(),
            refreshing: false,
            mgmt_pending: None,
            mgmt_title: String::new(),
            mgmt_for: None,
            data_revision: 0,
        }
    }

    /// Cancel the job running on the worker thread (Esc). Returns false when
    /// the app is idle.
    fn cancel_job(&mut self) -> bool {
        match &self.job {
            Some(job) => {
                job.cancel.cancel();
                self.message = format!("cancelling {}...", job.label);
                true
            }
            None => false,
        }
    }

    /// Inputs the cached management panel depends on.
    pub fn mgmt_key(&self) -> (usize, bool, u64) {
        (self.cursor_left, self.detail.is_some(), self.data_revision)
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

    /// Open the new-project modal with a fresh name, default template and no
    /// database, focused on the name field.
    pub fn begin_quick_create(&mut self) {
        self.quick_create = Some(QuickCreateField::Name);
        self.name_input.clear();
        self.template_cursor = default_template_index();
        self.with_db = false;
        self.form_error = None;
        self.message = "new project: type a name, then Enter".into();
    }

    fn cancel_quick_create(&mut self) {
        self.quick_create = None;
        self.name_input.clear();
        self.with_db = false;
        self.form_error = None;
        self.message = "create cancelled".into();
    }

    /// Validate the name, then queue the project creation. An invalid name keeps
    /// the form open on the name field with the reason shown inline.
    fn submit_quick_create(&mut self) {
        let name = self.name_input.trim().to_string();
        if let Err(e) = fine_ill_do_it_myself::project::validate_name(&name) {
            self.form_error = Some(e.to_string());
            self.quick_create = Some(QuickCreateField::Name);
            self.message = format!("invalid name: {e}");
            return;
        }
        let template = fine_ill_do_it_myself::project::Template::ALL[self.template_cursor];
        self.quick_create = None;
        self.name_input.clear();
        self.form_error = None;
        self.message = format!("creating {name} ({})...", template.label());
        self.actions.push(Action::CreateProject {
            name,
            template,
            db: self.with_db,
        });
        self.with_db = false;
    }

    /// Pure transition for a key event.
    pub fn on_key(&mut self, key: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        // The new-project form owns every key while it is open.
        if let Some(field) = self.quick_create {
            match key {
                KeyCode::Esc => {
                    self.cancel_quick_create();
                    return;
                }
                KeyCode::Tab => {
                    self.quick_create = Some(field.next());
                    return;
                }
                KeyCode::BackTab => {
                    self.quick_create = Some(field.prev());
                    return;
                }
                KeyCode::Enter => {
                    self.submit_quick_create();
                    return;
                }
                _ => {}
            }
            match field {
                QuickCreateField::Name => match key {
                    KeyCode::Backspace => {
                        self.name_input.pop();
                        self.form_error = None;
                    }
                    KeyCode::Char(c) if name_char(c) => {
                        if self.name_input.len() < 63 {
                            self.name_input.push(c);
                        }
                        self.form_error = None;
                    }
                    _ => {}
                },
                QuickCreateField::Template => match key {
                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Left | KeyCode::Char('h') => {
                        self.template_cursor = self.template_cursor.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Right | KeyCode::Char('l') => {
                        self.template_cursor = (self.template_cursor + 1)
                            .min(fine_ill_do_it_myself::project::Template::ALL.len() - 1);
                    }
                    KeyCode::Char('d') => self.with_db = !self.with_db,
                    _ => {}
                },
                QuickCreateField::Database => match key {
                    KeyCode::Char(' ') | KeyCode::Char('d') | KeyCode::Char('x') => {
                        self.with_db = !self.with_db;
                    }
                    _ => {}
                },
            }
            return;
        }
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
        // A running job owns Esc: it cancels the work instead of quitting.
        if key == KeyCode::Esc && self.cancel_job() {
            return;
        }
        if key == self.keybinds.quit {
            self.quit = true;
            return;
        }
        // `n` starts the in-TUI quick-create flow from the projects panel.
        if key == KeyCode::Char('n')
            && !self.panel_right
            && focused_panel(self) == "projects"
            && self.quick_create.is_none()
        {
            self.begin_quick_create();
            return;
        }
        if key == self.keybinds.editor {
            // `o` opens the project folder in the code editor; it only makes
            // sense on the projects container (the browser stays on Enter).
            if !self.panel_right && focused_panel(self) == "projects" {
                let idx = self.cursor().saturating_sub(self.services.len());
                if let Some(p) = self.projects.get(idx).cloned() {
                    self.message = format!("opening {p} in editor...");
                    self.actions.push(Action::OpenEditor(p));
                    return;
                }
            }
            self.message = "select a project first (tab to the projects panel)".into();
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
                // Language rows open their version list; Enter never toggles a runtime.
                "languages" => {
                    let i = self.cursor();
                    let name = self.services[i].name.clone();
                    if is_language(&name) {
                        self.detail = Some(format!("detail: {name}"));
                        self.panel_right = true;
                        self.cursor_right = 0;
                        self.message = "select a version, Enter to activate".into();
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
        if let Some(v) = text.strip_prefix("composer.install") {
            let v = v.trim();
            self.message = format!("composer.install {v} requested");
            self.actions.push(Action::InstallComposer);
        } else if let Some(version) = text.strip_prefix("php.install ") {
            self.message = format!("php.install {version} requested");
            self.actions.push(Action::InstallPhp(version.to_string()));
        } else if let Some(rest) = text.strip_prefix("mariadb.install ") {
            self.message = format!("mariadb.install {} requested", rest.trim());
            self.actions
                .push(Action::InstallManaged("mariadb".to_string()));
        } else if let Some(rest) = text.strip_prefix("phpmyadmin.install ") {
            self.message = format!("phpmyadmin.install {} requested", rest.trim());
            self.actions
                .push(Action::InstallManaged("phpmyadmin".to_string()));
        } else if let Some((server, version)) = text.split_once(".install ") {
            if [
                fine_ill_do_it_myself::orchestrator::CADDY,
                fine_ill_do_it_myself::orchestrator::APACHE,
                fine_ill_do_it_myself::orchestrator::NGINX,
                fine_ill_do_it_myself::orchestrator::LIGHTTPD,
            ]
            .contains(&server)
            {
                self.message = format!("{server}.install requested");
                self.actions.push(Action::InstallWeb(server.to_string()));
            } else if ["node", "python", "go", "java"].contains(&server) {
                self.message = format!("{server}.install {version} requested");
                self.actions.push(Action::InstallLang {
                    lang: server.to_string(),
                    version: version.to_string(),
                });
            }
        } else if let Some(v) = text.strip_prefix("php.use ") {
            self.message = format!("php.use {v} requested");
            self.actions.push(Action::UsePhp(v.to_string()));
        } else if let Some(n) = text.strip_prefix("web.use ") {
            let server = n.split_whitespace().next().unwrap_or_default();
            self.message = format!("web.use {server} requested");
            self.actions.push(Action::UseWeb(server.to_string()));
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
                self.message = "Enter to apply, Esc to cancel".into();
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
                } else if other == Container::Projects {
                    // Keep an explicit focus position even while the projects
                    // list is empty, so `n` can create its first entry.
                    self.cursor_left = self.services.len();
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
            .filter(|c| {
                *c == Container::Management
                    || *c == Container::Projects
                    || !self.container_rows(*c).is_empty()
            })
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
        // The rendered management panel is a dynamic, cached list: installed
        // runtimes, installable versions, composer, ports, managed components.
        // Its own line count is the only source of truth for the cursor. Deriving
        // it from `config_rows` (as PHP and MariaDB used to) hid the extra rows
        // and trapped navigation on the first line.
        if self.mgmt_line_count == 0 && self.mgmt_lines.is_empty() {
            // The panel is still loading; keep the cursor usable until the
            // worker publishes the real row count.
            return self.config_rows.len().max(1);
        }
        self.mgmt_line_count.max(self.mgmt_lines.len()).max(1)
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
        } else if cur < non_lang_count + lang_count + app.projects.len()
            || (app.projects.is_empty() && cur == non_lang_count + lang_count)
        {
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
        ver.to_string()
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
            ("backspace".to_string(), "delete"),
            ("enter".to_string(), "apply"),
            ("esc".to_string(), "cancel"),
        ];
    }
    if app.quick_create.is_some() {
        return vec![
            ("tab".to_string(), "field"),
            ("↑/↓".to_string(), "template"),
            ("space".to_string(), "database"),
            ("enter".to_string(), "create"),
            ("esc".to_string(), "cancel"),
        ];
    }
    let fixed = [
        (disp(app.keybinds.quit), "quit"),
        ("j".to_string(), "down"),
        ("k".to_string(), "up"),
        ("d".to_string(), "detail"),
        ("enter".to_string(), "apply"),
        ("tab".to_string(), "panel"),
        (disp(app.keybinds.editor), "edit"),
        (disp(app.keybinds.start), "start"),
        (disp(app.keybinds.stop), "stop"),
        (disp(app.keybinds.restart), "restart"),
        (disp(app.keybinds.restart_all), "all"),
    ];
    let mut fixed = fixed.to_vec();
    // The new-project form is only reachable from the projects panel, so the
    // hint belongs there and nowhere else.
    if focused_panel(app) == "projects" && app.quick_create.is_none() {
        // First, so the footer never clips it behind the fixed service keys.
        fixed.insert(0, ("n".to_string(), "new project"));
    }
    if app.job.is_some() {
        fixed.insert(0, ("esc".to_string(), "cancel"));
    }
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for (k, l) in fixed {
        if !out.iter().any(|(seen, _)| seen == &k) {
            out.push((k, l));
        }
    }
    out
}

fn web_install_rows(env: &fine_ill_do_it_myself::Env, server: &str) -> Vec<String> {
    if fine_ill_do_it_myself::orchestrator::web_bin_path(env, server).is_file() {
        return Vec::new();
    }

    let manifest = effective_manifest(env);
    manifest
        .find(server)
        .map(|source| vec![format!("  {server}.install {}", source.version)])
        .unwrap_or_default()
}

fn effective_manifest(env: &fine_ill_do_it_myself::Env) -> fine_ill_do_it_myself::Manifest {
    let defaults =
        fine_ill_do_it_myself::Manifest::parse(fine_ill_do_it_myself::installer_defaults())
            .expect("bundled installer manifest must be valid");
    let mut manifest =
        fine_ill_do_it_myself::Manifest::load(&env.manifest_path()).unwrap_or_default();
    for source in defaults.binaries {
        if !manifest
            .binaries
            .iter()
            .any(|existing| existing.name == source.name)
        {
            manifest.binaries.push(source);
        }
    }
    manifest
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
fn mgmt_panel_for(
    env: &fine_ill_do_it_myself::Env,
    services: &[ServiceStatus],
    cursor_left: usize,
    config_rows: &[ConfigRow],
) -> (String, Vec<String>) {
    let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
    let cur_php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
    let cur_web = fine_ill_do_it_myself::orchestrator::web_server(env);
    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());

    if let Some(svc) = services.get(cursor_left) {
        if svc.name.starts_with("php-") || svc.name == "php" {
            let mut lines: Vec<String> =
                fine_ill_do_it_myself::orchestrator::installed_php_versions(env)
                    .into_iter()
                    .map(|v| {
                        let mark = if v == cur_php { "▶ " } else { "  " };
                        format!("{mark}php.use {v}")
                    })
                    .collect();
            lines.extend(php_install_rows(env));
            lines.push(composer_row(env));
            return (format!(" php versions (current: {cur_php}) "), lines);
        }
        for lang in ["node", "python", "go", "java"] {
            if svc.name == lang || svc.name.starts_with(&format!("{lang}-")) {
                let cur = fine_ill_do_it_myself::lang::active_lang_version(env, lang)
                    .unwrap_or_else(|| "none".into());
                let versions = fine_ill_do_it_myself::lang::installed_lang_versions(env, lang);
                let mut lines = Vec::new();
                if versions.is_empty() {
                    lines.push("  no versions installed:".to_string());
                } else {
                    for v in &versions {
                        let is_active = v == &cur
                            || (v == "system" && (cur == "system" || cur.starts_with("system")));
                        let mark = if is_active { "▶ " } else { "  " };
                        lines.push(format!("{mark}{lang}.use {v}"));
                    }
                }
                lines.extend(lang_install_rows(env, lang));
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
            lines.extend(web_install_rows(env, &svc.name));
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
            let mut lines = vec![
                format!("  port http = {}", ports.http),
                format!("  port https = {}", ports.https),
                format!("  port fpm_base = {}", ports.fpm_base),
                format!("  port mariadb = {}", ports.mariadb),
            ];
            lines.extend(managed_install_rows(env));
            return (" ports ".to_string(), lines);
        }
    }
    // no service focused: the global config list
    let mut lines: Vec<String> = config_rows
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
        .collect();
    for server in [
        fine_ill_do_it_myself::orchestrator::CADDY,
        fine_ill_do_it_myself::orchestrator::APACHE,
        fine_ill_do_it_myself::orchestrator::NGINX,
        fine_ill_do_it_myself::orchestrator::LIGHTTPD,
    ] {
        lines.extend(web_install_rows(env, server));
    }
    lines.push(composer_row(env));
    lines.extend(php_install_rows(env));
    for lang in ["node", "python", "go", "java"] {
        lines.extend(lang_install_rows(env, lang));
    }
    lines.extend(managed_install_rows(env));
    (" config (select service) ".to_string(), lines)
}

/// Management row for the managed composer.phar: version plus install action
/// when it is missing.
fn composer_row(env: &fine_ill_do_it_myself::Env) -> String {
    let version =
        fine_ill_do_it_myself::composer::pinned_version(env).unwrap_or_else(|| "?".to_string());
    if fine_ill_do_it_myself::composer::is_installed(env) {
        format!("  composer {version} [installed]")
    } else {
        format!("  composer.install {version}")
    }
}

fn php_install_rows(env: &fine_ill_do_it_myself::Env) -> Vec<String> {
    let manifest =
        fine_ill_do_it_myself::Manifest::parse(fine_ill_do_it_myself::installer_defaults())
            .expect("bundled installer manifest must be valid");
    let installed = fine_ill_do_it_myself::orchestrator::installed_php_versions(env);
    manifest
        .binaries
        .iter()
        .filter_map(|binary| {
            let version = binary.name.strip_prefix("php-fpm-")?;
            manifest.find(&format!("php-{version}"))?;
            (!installed.iter().any(|current| current == version))
                .then(|| format!("  php.install {version}"))
        })
        .collect()
}

fn lang_install_rows(env: &fine_ill_do_it_myself::Env, lang: &str) -> Vec<String> {
    let manifest =
        fine_ill_do_it_myself::Manifest::parse(fine_ill_do_it_myself::installer_defaults())
            .expect("bundled installer manifest must be valid");
    let prefix = format!("{lang}-");
    let installed = fine_ill_do_it_myself::lang::installed_lang_versions(env, lang);
    manifest
        .binaries
        .iter()
        .filter_map(|binary| {
            let version = binary.name.strip_prefix(&prefix)?;
            (!installed.iter().any(|current| current == version))
                .then(|| format!("  {lang}.install {version}"))
        })
        .collect()
}

fn managed_install_rows(env: &fine_ill_do_it_myself::Env) -> Vec<String> {
    let manifest =
        fine_ill_do_it_myself::Manifest::parse(fine_ill_do_it_myself::installer_defaults())
            .expect("bundled installer manifest must be valid");
    let mut rows = Vec::new();
    if fine_ill_do_it_myself::orchestrator::find_mariadbd(env).is_none() {
        if let Some(binary) = manifest.find("mariadb") {
            rows.push(format!("  mariadb.install {}", binary.version));
        }
    }
    if fine_ill_do_it_myself::phpmyadmin::install_dir(env).is_none() {
        if let Some(binary) = manifest.find("phpmyadmin") {
            rows.push(format!("  phpmyadmin.install {}", binary.version));
        }
    }
    rows
}

/// Template preselected in the modal: a plain PHP page needs no Composer or
/// network, so the first project always works.
fn default_template_index() -> usize {
    fine_ill_do_it_myself::project::Template::ALL
        .iter()
        .position(|t| *t == fine_ill_do_it_myself::project::Template::Php)
        .unwrap_or(0)
}

/// Characters accepted while typing a project name (DNS label subset).
fn name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Centered modal rectangle, clamped so it keeps a sane size on small
/// terminals without ever exceeding the screen.
fn quick_create_modal(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let width = area.width.saturating_sub(4).min(80);
    let height = area.height.saturating_sub(2).min(20);
    ratatui::layout::Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// Draw the new-project modal form: name input, template list, database
/// checkbox, inline validation and key hints. Extracted from the event loop so
/// tests can render it on a `TestBackend` without a real terminal.
pub fn render_quick_create(app: &App, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::{Alignment, Constraint, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph,
    };

    let Some(field) = app.quick_create else {
        return;
    };
    let modal = quick_create_modal(area);
    // Hide whatever is behind the form so it reads as one dialog.
    frame.render_widget(Clear, modal);
    let outer = Block::default()
        .title(Span::styled(
            " New project ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(modal);
    frame.render_widget(outer, modal);

    let [name_area, hint_area, template_title_area, templates_area, db_area, error_area, help_area] =
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(7),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);

    // Name input -------------------------------------------------------
    let name_focused = field == QuickCreateField::Name;
    frame.render_widget(
        Paragraph::new(app.name_input.as_str()).block(
            Block::default()
                .title(" Name ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if name_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })),
        ),
        name_area,
    );
    if name_focused {
        let x = name_area.x + 1 + app.name_input.len() as u16;
        let y = name_area.y + 1;
        if x < name_area.x + name_area.width.saturating_sub(1) {
            frame.set_cursor_position((x, y));
        }
    }
    let valid = fine_ill_do_it_myself::project::validate_name(&app.name_input).is_ok();
    let hint = if app.name_input.is_empty() {
        Line::from(Span::styled(
            " a-z, 0-9 and - · the site opens at https://<name>.test",
            Style::default().fg(Color::DarkGray),
        ))
    } else if valid {
        Line::from(vec![
            Span::styled(" → ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("https://{}.test", app.name_input),
                Style::default().fg(Color::Green),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " a-z, 0-9 and - only",
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(hint), hint_area);

    // Template list ----------------------------------------------------
    let template_focused = field == QuickCreateField::Template;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Template",
                Style::default()
                    .fg(if template_focused {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ↑/↓ to choose", Style::default().fg(Color::DarkGray)),
        ])),
        template_title_area,
    );
    let template_items: Vec<ListItem> = quick_create_template_options()
        .into_iter()
        .map(|(template, description)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<16}", template.label()),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(description, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();
    let mut template_state = ListState::default();
    template_state.select(Some(app.template_cursor));
    frame.render_stateful_widget(
        List::new(template_items)
            .highlight_symbol(" ▸ ")
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        templates_area,
        &mut template_state,
    );

    // Database checkbox -------------------------------------------------
    let db_focused = field == QuickCreateField::Database;
    let db_style = if db_focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if app.with_db {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Gray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if db_focused { " ▸ " } else { "   " },
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                if app.with_db { "[x] " } else { "[ ] " },
                Style::default().fg(if app.with_db {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled("Create a MariaDB database with the same name", db_style),
        ])),
        db_area,
    );

    // Inline error + key hints ------------------------------------------
    let error = app
        .form_error
        .as_ref()
        .map(|e| {
            Line::from(Span::styled(
                format!(" ⚠ {e}"),
                Style::default().fg(Color::Red),
            ))
        })
        .unwrap_or_default();
    frame.render_widget(Paragraph::new(error), error_area);
    frame.render_widget(
        Paragraph::new(" Tab: field · ↑/↓: template · Space: db · Enter: create · Esc: cancel")
            .style(Style::default().fg(Color::DarkGray)),
        help_area,
    );
}

/// Template choices and concise descriptions shown in the quick-create list.
fn quick_create_template_options() -> Vec<(fine_ill_do_it_myself::project::Template, &'static str)>
{
    use fine_ill_do_it_myself::project::Template;

    Template::ALL
        .into_iter()
        .map(|template| {
            let description = match template {
                Template::Laravel => "Laravel application · Composer and internet required",
                Template::Wordpress => "WordPress site · download required",
                Template::Php => "PHP welcome page · index.php",
                Template::Html => "Static website · index.html",
                Template::Blank => "Empty project directory",
                Template::Node => "Node.js HTTP server · reverse proxy on port 3000",
                Template::Python => "Python HTTP server · reverse proxy on port 8000",
            };
            (template, description)
        })
        .collect()
}

/// How long one `poll` may block waiting for a key.
const TICK: std::time::Duration = std::time::Duration::from_millis(100);
/// Status refresh cadence while the UI is idle.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
/// How often the spinner is repainted while a job runs.
const SPINNER_TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// Set from SIGTERM/SIGINT/SIGHUP so the loop can restore the terminal.
static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn request_stop(_: libc::c_int) {
    STOP_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn stop_requested() -> bool {
    STOP_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Put the terminal back the way we found it. Safe to call twice.
fn restore_terminal() {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
}

/// A panic, SIGTERM or SIGINT must never leave the user in raw mode with no
/// cursor. Ctrl-C cannot reach us as a signal while raw mode is on, so the
/// signal handlers only matter for `kill` / window-manager shutdown.
fn install_terminal_hooks() {
    let main_thread = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A worker panic must not tear down the screen under a live UI.
        if std::thread::current().id() == main_thread {
            restore_terminal();
        }
        previous(info);
    }));
    unsafe {
        libc::signal(
            libc::SIGTERM,
            request_stop as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            request_stop as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            request_stop as *const () as libc::sighandler_t,
        );
    }
}

/// One long-running action running on the worker thread.
pub struct Job {
    pub label: String,
    pub started: std::time::Instant,
    pub cancel: fine_ill_do_it_myself::CancelToken,
}

impl Job {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            started: std::time::Instant::now(),
            cancel: fine_ill_do_it_myself::CancelToken::new(),
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }
}

/// Messages the worker thread sends back to the UI loop.
enum WorkerMsg {
    /// Download progress as (done, total); total 0 means "unknown".
    Progress {
        done: u64,
        total: u64,
    },
    Done(Outcome),
    RefreshDone(std::result::Result<RefreshData, String>),
    ManagementDone {
        key: (usize, bool, u64),
        result: std::result::Result<(String, Vec<String>), String>,
    },
}

struct RefreshData {
    services: Vec<ServiceStatus>,
    language_reps: Vec<usize>,
    ports: fine_ill_do_it_myself::PortPlan,
    php_row: String,
    lang_versions: std::collections::BTreeMap<String, String>,
    config_rows: Vec<ConfigRow>,
}

/// What the UI has to apply once an action finished. Everything that needs the
/// terminal (a terminal editor) is handed back to the loop instead of running
/// inside the worker.
struct Outcome {
    message: String,
    detail: Option<String>,
    created_project: Option<String>,
    terminal_editor: Option<Vec<String>>,
}

/// Read-only view of the app that the worker needs. Captured before spawning so
/// the worker never touches UI state.
struct ActionContext {
    services: Vec<String>,
    editor: Option<fine_ill_do_it_myself::editor::Editor>,
}

fn action_label(action: &Action, ctx: &ActionContext) -> String {
    let svc = |i: usize| {
        ctx.services
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("#{i}"))
    };
    match action {
        Action::RestartAll => "restarting everything".into(),
        Action::StartService(i) => format!("starting {}", svc(*i)),
        Action::StopService(i) => format!("stopping {}", svc(*i)),
        Action::RestartService(i) => format!("restarting {}", svc(*i)),
        Action::UsePhp(v) => format!("switching php to {v}"),
        Action::UseWeb(n) => format!("switching web server to {n}"),
        Action::SetPort { key, value } => format!("setting {key} = {value}"),
        Action::ShowDetail(n) => format!("loading {n}"),
        Action::UseServerVersion { server, version } => format!("switching {server} to {version}"),
        Action::InstallWeb(server) => format!("installing {server}"),
        Action::UseLang { lang, version } => format!("switching {lang} to {version}"),
        Action::OpenProject(p) => format!("opening {p}.test"),
        Action::OpenEditor(p) => format!("opening {p} in the editor"),
        Action::InstallComposer => "downloading composer".into(),
        Action::InstallPhp(version) => format!("installing PHP {version}"),
        Action::InstallLang { lang, version } => format!("installing {lang} {version}"),
        Action::InstallManaged(name) => format!("installing {name}"),
        Action::CreateProject { name, .. } => format!("creating {name}"),
    }
}

/// Hand one action to a worker thread. The UI thread returns immediately, so
/// the screen keeps drawing and keys keep working during a slow download or a
/// MariaDB first start.
fn start_job(
    env: &fine_ill_do_it_myself::Env,
    app: &mut App,
    action: Action,
    tx: std::sync::mpsc::Sender<WorkerMsg>,
) {
    let ctx = ActionContext {
        services: app.services.iter().map(|s| s.name.clone()).collect(),
        editor: app.editor.clone(),
    };
    let job = Job::new(action_label(&action, &ctx));
    let cancel = job.cancel.clone();
    let label = job.label.clone();
    app.job = Some(job);
    app.progress = None;
    app.message.clear();
    let env = env.clone();
    let spawned = std::thread::Builder::new()
        .name("fidim-job".into())
        .spawn(move || {
            let progress = tx.clone();
            let fetcher = fine_ill_do_it_myself::installer::CurlFetcher::with_progress(
                cancel.clone(),
                move |done, total| {
                    let _ = progress.send(WorkerMsg::Progress { done, total });
                },
            );
            // A panic in one action must surface as a message, never as a
            // spinner that runs forever with no way out.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute_action(
                    &env,
                    &fine_ill_do_it_myself::RealRunner,
                    &ctx,
                    &action,
                    &fetcher,
                )
            }))
            .unwrap_or_else(|_| Outcome {
                message: format!("{label}: internal error (see logs)"),
                detail: None,
                created_project: None,
                terminal_editor: None,
            });
            let _ = tx.send(WorkerMsg::Done(outcome));
        });
    if let Err(e) = spawned {
        app.job = None;
        app.message = format!("cannot start worker: {e}");
    }
}

fn apply_outcome(app: &mut App, outcome: Outcome) {
    app.message = outcome.message;
    if let Some(name) = outcome.created_project {
        app.projects.push(name);
        app.projects.sort();
        app.projects.dedup();
    }
    if let Some(cmd) = outcome.terminal_editor {
        app.pending_terminal_editor = Some(cmd);
    }
    if let Some(detail) = outcome.detail {
        app.detail = Some(detail);
        app.panel_right = true;
        app.cursor_right = 0;
    }
}

fn stop_named<R>(
    env: &fine_ill_do_it_myself::Env,
    ports: &fine_ill_do_it_myself::PortPlan,
    runner: &R,
    name: &str,
) -> fine_ill_do_it_myself::Result<()>
where
    R: fine_ill_do_it_myself::Runner + Clone,
{
    fine_ill_do_it_myself::supervisor::Supervisor {
        env: env.clone(),
        ports: ports.clone(),
        runner: runner.clone(),
    }
    .stop(name)
}

/// Detail text for one service row (reads lock/bin state; worker thread only).
fn service_detail(env: &fine_ill_do_it_myself::Env, name: &str) -> String {
    let mut detail = format!("Service: {name}\n\n");
    if name.starts_with("php-") {
        detail.push_str("PHP-FPM Service\n");
        if let Ok(lock) = fine_ill_do_it_myself::Lock::load(&env.lock_path()) {
            let cur = fine_ill_do_it_myself::orchestrator::default_php(&lock);
            detail.push_str(&format!("Current default: {cur}\n"));
        }
        if let Some(ver) = name.strip_prefix("php-") {
            if let Some(bin) = fine_ill_do_it_myself::orchestrator::php_binary_for(env, ver) {
                detail.push_str(&format!("Binary: {}\n", bin.display()));
            }
        }
    } else if ["caddy", "httpd", "nginx", "lighttpd"].contains(&name) {
        detail.push_str("Web Server\n");
        if let Some(ver) = fine_ill_do_it_myself::orchestrator::web_version(env, name) {
            detail.push_str(&format!("Version: {ver}\n"));
        }
        let ports =
            fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        let (http, https) = ports.web_ports(name);
        detail.push_str(&format!("HTTP port: {http}\n"));
        detail.push_str(&format!("HTTPS port: {https}\n"));
    } else if name == "mariadb" {
        detail.push_str("MariaDB Database\n");
        let ports =
            fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        detail.push_str(&format!("Port: {}\n", ports.mariadb));
    }
    detail
}

/// Execute one action. Runs on the worker thread (or synchronously from tests)
/// and returns everything the UI has to apply, never touching `App`.
fn execute_action<R, F>(
    env: &fine_ill_do_it_myself::Env,
    runner: &R,
    ctx: &ActionContext,
    action: &Action,
    fetcher: &F,
) -> Outcome
where
    R: fine_ill_do_it_myself::Runner + Clone,
    F: fine_ill_do_it_myself::installer::Fetcher,
{
    let ports =
        fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
    let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
    let php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
    let mut detail = None;
    let mut created_project = None;
    let mut terminal_editor = None;

    let r: std::result::Result<(), fine_ill_do_it_myself::Error> = match action {
        Action::RestartAll => fine_ill_do_it_myself::orchestrator::stop_all(env, &ports, runner)
            .and_then(|_| fine_ill_do_it_myself::orchestrator::start_all(env, &ports, runner, &php))
            .map(|_| ()),
        Action::StartService(i) => match ctx.services.get(*i) {
            Some(name) => {
                fine_ill_do_it_myself::orchestrator::start_service(env, &ports, runner, &php, name)
                    .map(|_| ())
            }
            None => Err(fine_ill_do_it_myself::Error::Other(format!(
                "service row {i} no longer exists"
            ))),
        },
        Action::StopService(i) => match ctx.services.get(*i) {
            Some(name) => stop_named(env, &ports, runner, name),
            None => Err(fine_ill_do_it_myself::Error::Other(format!(
                "service row {i} no longer exists"
            ))),
        },
        Action::RestartService(i) => match ctx.services.get(*i) {
            Some(name) => stop_named(env, &ports, runner, name).and_then(|_| {
                fine_ill_do_it_myself::orchestrator::start_service(env, &ports, runner, &php, name)
                    .map(|_| ())
            }),
            None => Err(fine_ill_do_it_myself::Error::Other(format!(
                "service row {i} no longer exists"
            ))),
        },
        Action::UsePhp(v) => {
            let cli_env = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: v.clone(),
            };
            crate::cli::php_use(&cli_env, v)
        }
        Action::UseLang { lang, version } => {
            let cli_env = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: version.clone(),
            };
            crate::cli::lang_use(&cli_env, lang, version)
        }
        Action::UseWeb(name) => {
            if fine_ill_do_it_myself::orchestrator::resolve_web_bin(env, name).is_err() {
                Err(fine_ill_do_it_myself::Error::Other(format!(
                    "web server {name} not installed"
                )))
            } else if let Ok(mut lock) = fine_ill_do_it_myself::Lock::load(&env.lock_path()) {
                lock.web_default = Some(name.clone());
                let php = fine_ill_do_it_myself::orchestrator::default_php(&lock);
                lock.save(&env.lock_path())
                    .map_err(fine_ill_do_it_myself::Error::Io)
                    .and_then(|_| {
                        fine_ill_do_it_myself::orchestrator::generate_configs(env, &ports, &php)
                    })
            } else {
                Err(fine_ill_do_it_myself::Error::Other(
                    "failed to load lock".into(),
                ))
            }
        }
        Action::SetPort { key, value } => crate::cli::port_set(&env.paths.etc(), key, *value),
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
                res.map_err(fine_ill_do_it_myself::Error::Io)
            }
        }
        Action::ShowDetail(name) => {
            detail = Some(service_detail(env, name));
            Ok(())
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
                Ok(())
            }
        }
        Action::OpenEditor(project) => {
            let site = env.paths.sites().join(project);
            if !site.is_dir() {
                Err(fine_ill_do_it_myself::Error::Other(format!(
                    "project folder {} not found",
                    site.display()
                )))
            } else if let Some(editor) = ctx.editor.clone() {
                let cmd = editor.command(&site);
                if editor.kind == fine_ill_do_it_myself::editor::EditorKind::Terminal {
                    // the event loop owns the terminal: park the argv for it
                    terminal_editor = Some(cmd);
                    Ok(())
                } else {
                    runner
                        .spawn(&cmd)
                        .map(|_| ())
                        .map_err(fine_ill_do_it_myself::Error::Io)
                }
            } else {
                Err(fine_ill_do_it_myself::Error::Other(
                    "no code editor found; set FIDIM_EDITOR (e.g. FIDIM_EDITOR=code)".into(),
                ))
            }
        }
        Action::InstallComposer => {
            let cli = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: php.clone(),
            };
            crate::cli::load_or_default_manifest(&cli).and_then(|manifest| {
                let mut lock =
                    fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
                fine_ill_do_it_myself::composer::ensure(env, &manifest, &mut lock, fetcher)
                    .map(|_| ())
                    .and_then(|_| crate::cli::install_shims(&cli).map(|_| ()))
            })
        }
        Action::InstallPhp(version) => {
            let cli = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: php.clone(),
            };
            crate::cli::php_install(&cli, version).and_then(|_| crate::cli::php_use(&cli, version))
        }
        Action::InstallLang { lang, version } => {
            let cli = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: php.clone(),
            };
            crate::cli::lang_install(&cli, lang, version)
                .and_then(|_| crate::cli::lang_use(&cli, lang, version))
        }
        Action::InstallManaged(name) => {
            if !["mariadb", "phpmyadmin"].contains(&name.as_str()) {
                Err(fine_ill_do_it_myself::Error::Other(format!(
                    "unsupported managed component: {name}"
                )))
            } else {
                let cli = crate::cli::CliEnv {
                    env: env.clone(),
                    ports: ports.clone(),
                    php_default: php.clone(),
                };
                crate::cli::install(&cli, name)
                    .and_then(|_| crate::cli::install_shims(&cli).map(|_| ()))
            }
        }
        Action::InstallWeb(server) => {
            let cli = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: php.clone(),
            };
            crate::cli::install(&cli, server).map(|_| ())
        }
        Action::CreateProject { name, template, db } => {
            let cli = crate::cli::CliEnv {
                env: env.clone(),
                ports: ports.clone(),
                php_default: php.clone(),
            };
            crate::cli::create_project_with_fetcher(&cli, name, *template, *db, fetcher).map(
                |made| {
                    created_project = Some(
                        made.path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                },
            )
        }
    };

    let failed = r.is_err();
    let message = match (&r, action) {
        (Ok(()), Action::OpenProject(p)) => {
            let web = fine_ill_do_it_myself::orchestrator::default_web(&lock);
            let (http_port, _) = ports.web_ports(&web);
            let port_str = if http_port == 80 {
                String::new()
            } else {
                format!(":{http_port}")
            };
            format!("opened http://{p}.test{port_str} in browser")
        }
        (Ok(()), Action::OpenEditor(p)) => {
            let editor = ctx
                .editor
                .as_ref()
                .map(|e| e.name())
                .unwrap_or_else(|| "editor".into());
            format!("opening {p} in {editor}")
        }
        (Ok(()), Action::InstallComposer) => {
            "composer installed; PATH setup: fidim env --install --shell".into()
        }
        (Ok(()), Action::InstallPhp(version)) => {
            format!(
                "PHP {version} installed and activated; PATH setup: fidim env --install --shell"
            )
        }
        (Ok(()), Action::InstallLang { lang, version }) => {
            format!(
                "{lang} {version} installed and activated; PATH setup: fidim env --install --shell"
            )
        }
        (Ok(()), Action::InstallManaged(name)) => {
            format!("{name} installed; PATH setup: fidim env --install --shell")
        }
        (Ok(()), Action::InstallWeb(server)) => format!("{server} installed"),
        (Ok(()), Action::CreateProject { name, template, .. }) => {
            format!("created {name} ({})", template.label())
        }
        (Ok(()), _) => format!("{action:?} ok").to_lowercase(),
        (Err(e), Action::OpenEditor(p)) => format!("{p}: {e}"),
        (Err(e), Action::InstallComposer) => format!("composer install: {e}"),
        (Err(e), Action::InstallPhp(version)) => format!("PHP {version} install: {e}"),
        (Err(e), Action::InstallLang { lang, version }) => {
            format!("{lang} {version} install: {e}")
        }
        (Err(e), Action::InstallManaged(name)) => format!("{name} install: {e}"),
        (Err(e), Action::InstallWeb(server)) => format!("{server} install: {e}"),
        (Err(e), Action::CreateProject { name, .. }) => format!("create {name}: {e}"),
        (Err(e), _) => format!("{action:?}: {e}").to_lowercase(),
    };

    Outcome {
        message,
        detail: if failed { None } else { detail },
        created_project: if failed { None } else { created_project },
        terminal_editor: if failed { None } else { terminal_editor },
    }
}

/// Synchronous helper kept for tests (and `make_actioner`): execute one action
/// against the real system and apply its outcome immediately.
#[allow(dead_code)]
fn perform_action<R: fine_ill_do_it_myself::Runner + Clone>(
    env: &fine_ill_do_it_myself::Env,
    runner: R,
    app: &mut App,
    action: &Action,
) {
    let ctx = ActionContext {
        services: app.services.iter().map(|s| s.name.clone()).collect(),
        editor: app.editor.clone(),
    };
    let outcome = execute_action(
        env,
        &runner,
        &ctx,
        action,
        &fine_ill_do_it_myself::installer::CurlFetcher::default(),
    );
    apply_outcome(app, outcome);
}

/// Build the real-system action handler used by tests.
#[allow(dead_code)]
fn make_actioner(env: fine_ill_do_it_myself::Env) -> impl Fn(&mut App, &Action) {
    move |app: &mut App, action: &Action| {
        perform_action(&env.clone(), fine_ill_do_it_myself::RealRunner, app, action);
    }
}

/// Route one key: Enter on the right panel acts on the *cached* management
/// lines (so it matches exactly what is on screen), everything else goes to
/// `App::on_key`. Kept separate so the behaviour is testable without a tty.
pub fn handle_key(app: &mut App, key: crossterm::event::KeyCode) {
    if key == crossterm::event::KeyCode::Enter
        && app.panel_right
        && app.detail.is_some()
        && app.edit.is_none()
    {
        if app.mgmt_for == Some(app.mgmt_key()) {
            // Cloning cached lines avoids re-reading the filesystem for a
            // keystroke and guarantees Enter acts on the visible rows.
            let lines = app.mgmt_lines.clone();
            app.mgmt_enter(&lines);
        } else {
            app.message = "loading management options...".into();
        }
    } else {
        app.on_key(key);
    }
}

/// Next queued action, first-in first-out.
fn take_next(actions: &mut Vec<Action>) -> Option<Action> {
    if actions.is_empty() {
        None
    } else {
        Some(actions.remove(0))
    }
}

/// Spinner frame for the working indicator (one step per 250 ms).
pub fn spinner(elapsed: std::time::Duration) -> &'static str {
    const FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];
    FRAMES[(elapsed.as_millis() / 250) as usize % FRAMES.len()]
}

/// Pure rendering: reads only `App`, never the filesystem, a lock file or a
/// child process. Keeping I/O out of this function is what keeps the TUI
/// responsive; refresh work happens before the draw.
pub fn render(app: &App, f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};

    let (main_area, detail_area) = if app.detail.is_some() {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
                .areas(area);
        (left, Some(right))
    } else {
        (area, None)
    };

    let ports = &app.ports;
    let php_row = &app.php_row;

    let non_lang_indices: Vec<usize> = app
        .services
        .iter()
        .enumerate()
        .filter(|(_, s)| !is_language(&s.name))
        .map(|(i, _)| i)
        .collect();

    // Collapse language versions: show only one row per language family
    // (php, node, python, go, java). `language_reps` holds the active version
    // row for each family and is what navigation walks.
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

    // A status row (job / progress / message / port editor) gets its own line
    // so it can never be clipped by the key-hint row on a narrow terminal.
    let status_line = app.job.is_some()
        || app.progress.is_some()
        || !app.message.is_empty()
        || app.edit.is_some();

    let [services, languages, projects, footer] = Layout::vertical([
        Constraint::Length(svc_height),
        Constraint::Length(lang_height),
        Constraint::Min(1),
        Constraint::Length(if status_line { 2 } else { 1 }),
    ])
    .areas(main_area);

    let left_cursor = app.cursor_left;
    let svc_focus = !app.panel_right && non_lang_indices.contains(&left_cursor);
    let lang_focus = !app.panel_right && app.language_reps.contains(&left_cursor);
    let proj_focus = !app.panel_right && left_cursor >= app.services.len();

    // 1. Services panel (web servers, databases, ...)
    if svc_height > 0 {
        let items: Vec<ListItem> = non_lang_indices
            .iter()
            .map(|&idx| {
                let s = &app.services[idx];
                let (mark, name, color) = match &s.state {
                    ServiceState::Running => ("● ".to_string(), s.name.clone(), Color::Green),
                    ServiceState::Error(msg) => {
                        ("✗ ".to_string(), format!("{} ({msg})", s.name), Color::Red)
                    }
                    _ => ("○ ".to_string(), s.name.clone(), Color::DarkGray),
                };
                let p = ports_label(&s.name, php_row, ports);
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
                    ports_label(&s.name, php_row, ports)
                } else {
                    app.lang_versions
                        .get(display_name)
                        .cloned()
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
                        " programming languages{} ",
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
    let proj_title = format!(
        " projects (n: new project){} ",
        if proj_focus { " ◀" } else { "" }
    );
    let proj_list = List::new(proj_items)
        .highlight_symbol(
            if proj_focus && app.quick_create.is_none() {
                "▶ "
            } else {
                "  "
            }
            .to_owned(),
        )
        .highlight_style(Style::default().bg(Color::Blue))
        .block(
            Block::default()
                .title(proj_title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if proj_focus {
                    Color::Blue
                } else {
                    Color::DarkGray
                })),
        );
    f.render_stateful_widget(proj_list, projects, &mut plst);

    // Detail panel: exactly the lines cached by `refresh_mgmt`, so Enter on
    // the right panel acts on what is displayed.
    if let Some(detail_area) = detail_area {
        let mgmt_focus = app.panel_right;
        let mgmt_items: Vec<ListItem> = app
            .mgmt_lines
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
                    .title(app.mgmt_title.clone())
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

    // Footer row 1: working indicator, download progress, last message and the
    // port editor. Row 2 (when present) holds the context key hints.
    let (status_area, hint_area) = if status_line {
        let [status, hints] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(footer);
        (Some(status), hints)
    } else {
        (None, footer)
    };
    if let Some(status_area) = status_area {
        let mut spans: Vec<Span> = Vec::new();
        if let Some(job) = &app.job {
            spans.push(Span::styled(
                format!(" {} ", spinner(job.elapsed())),
                Style::default().fg(Color::Yellow),
            ));
            spans.push(Span::styled(
                format!("{} {:.0}s ", job.label, job.elapsed().as_secs_f32()),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if let Some((done, total)) = app.progress {
            spans.push(Span::styled(
                format!("{} ", progress_bar(done, total, 10)),
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
        f.render_widget(Paragraph::new(Line::from(spans)), status_area);
    }
    let mut spans: Vec<Span> = Vec::new();
    for (key, label) in footer_hints(app) {
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
    f.render_widget(Paragraph::new(Line::from(spans)), hint_area);
    render_quick_create(app, f, area);
}

/// Terminal entry: non-blocking event loop. Every system operation runs on a
/// worker thread; this loop only consumes messages, draws and reads keys, so a
/// slow download or a 15 s MariaDB first start can never freeze the interface.
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

    install_terminal_hooks();
    let _ = terminal::enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), terminal::EnterAlternateScreen);

    let projects: Vec<String> = fine_ill_do_it_myself::list_projects(env);
    let mut app = App::new(Vec::new(), projects);
    app.editor = fine_ill_do_it_myself::editor::detect_default();
    app.keybinds = load_keybinds(
        &std::fs::read_to_string(env.paths.etc().join("keys.toml")).unwrap_or_default(),
    );

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend).ok();

    let (tx, rx) = std::sync::mpsc::channel::<WorkerMsg>();
    start_refresh(env, &mut app, tx.clone());
    let mut draw = true;
    let mut last_draw = std::time::Instant::now();
    let mut last_refresh = std::time::Instant::now();

    loop {
        // 1. Results and progress from the worker thread.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMsg::Progress { done, total } => app.progress = Some((done, total)),
                WorkerMsg::Done(outcome) => {
                    apply_outcome(&mut app, outcome);
                    app.job = None;
                    app.progress = None;
                    // force the service list to refresh on the next step
                    last_refresh = std::time::Instant::now() - REFRESH_EVERY;
                }
                WorkerMsg::RefreshDone(result) => {
                    app.refreshing = false;
                    match result {
                        Ok(data) => apply_refresh(&mut app, data),
                        Err(error) => app.message = error,
                    }
                }
                WorkerMsg::ManagementDone { key, result } => {
                    apply_mgmt_refresh(&mut app, key, result);
                }
            }
            draw = true;
        }

        // 2. Hand the terminal to a terminal editor, then take it back.
        if app.job.is_none() {
            if let Some(cmd) = app.pending_terminal_editor.take() {
                restore_terminal();
                let status = std::process::Command::new(&cmd[0]).args(&cmd[1..]).status();
                let _ = crossterm::execute!(std::io::stdout(), terminal::EnterAlternateScreen);
                let _ = terminal::enable_raw_mode();
                if let Some(t) = term.as_mut() {
                    let _ = t.clear();
                }
                app.message = match status {
                    Ok(s) if s.success() => "editor closed".into(),
                    Ok(s) => format!("editor exited with {s}"),
                    Err(e) => format!("editor failed: {e}"),
                };
                draw = true;
            }
        }

        // 3. Periodic status refresh while the queue is empty.
        if app.job.is_none()
            && app.actions.is_empty()
            && !app.refreshing
            && last_refresh.elapsed() >= REFRESH_EVERY
        {
            start_refresh(env, &mut app, tx.clone());
            last_refresh = std::time::Instant::now();
        }

        // 4. Rebuild the management panel only when its inputs changed.
        if app.detail.is_some()
            && app.mgmt_for != Some(app.mgmt_key())
            && app.mgmt_pending.is_none()
        {
            start_mgmt_refresh(env, &mut app, tx.clone());
        }

        // 5. Dispatch the next queued action (FIFO, one job at a time).
        if app.job.is_none() {
            if let Some(action) = take_next(&mut app.actions) {
                start_job(env, &mut app, action, tx.clone());
                draw = true;
            }
        }

        // 6. Draw only when something changed (or to animate the spinner).
        if draw || (app.job.is_some() && last_draw.elapsed() >= SPINNER_TICK) {
            if let Some(t) = term.as_mut() {
                let _ = t.draw(|f| {
                    let area = f.area();
                    render(&app, f, area);
                });
            }
            last_draw = std::time::Instant::now();
            draw = false;
        }

        // 7. Input: drain every key already queued, then wait briefly.
        //    One key per 200 ms tick used to lag behind fast typing; a poll
        //    error (EINTR) must sleep instead of spinning at 100% CPU.
        let mut handled = 0;
        loop {
            let wait = if handled == 0 { TICK } else { Duration::ZERO };
            let ready = match crossterm::event::poll(wait) {
                Ok(ready) => ready,
                Err(_) => {
                    std::thread::sleep(TICK);
                    false
                }
            };
            if !ready {
                break;
            }
            match crossterm::event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    handle_key(&mut app, k.code);
                    // Moving the left cursor changes the right panel's rows;
                    // rebuild it before the next key so Enter can never act on
                    // a panel belonging to a different service.
                    if app.detail.is_some() && app.mgmt_for != Some(app.mgmt_key()) {
                        start_mgmt_refresh(env, &mut app, tx.clone());
                    }
                    draw = true;
                }
                Ok(Event::Resize(_, _)) => draw = true,
                Ok(_) => {}
                Err(_) => break,
            }
            handled += 1;
            if handled >= 64 || app.quit {
                break;
            }
        }

        if app.quit || stop_requested() {
            break;
        }
    }

    // A cancelled job may still hold a curl child: give it a moment to die so
    // nothing keeps downloading after the UI is gone.
    if let Some(job) = &app.job {
        job.cancel.cancel();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while app.job.is_some() && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(WorkerMsg::Done(outcome)) => {
                    apply_outcome(&mut app, outcome);
                    app.job = None;
                }
                Ok(WorkerMsg::Progress { .. }) => {}
                Ok(WorkerMsg::RefreshDone(_)) | Ok(WorkerMsg::ManagementDone { .. }) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
    restore_terminal();
    Ok(())
}

fn make_refresh(env: fine_ill_do_it_myself::Env) -> impl Fn() -> RefreshData {
    move || {
        let ports =
            fine_ill_do_it_myself::PortPlan::resolve(&env.paths.etc(), |k| std::env::var(k).ok());
        let mut statuses = fine_ill_do_it_myself::orchestrator::status_all(
            &env,
            &ports,
            &fine_ill_do_it_myself::RealRunner,
        );
        statuses.sort_by_key(|s| is_language(&s.name));
        let language_reps = language_representatives(&env, &statuses);

        let lock = fine_ill_do_it_myself::Lock::load(&env.lock_path()).unwrap_or_default();
        let php_row = fine_ill_do_it_myself::orchestrator::default_php(&lock);

        // Runtime versions are probed here, once per refresh, and cached for
        // the renderer. Probing inside `render` spawned processes per frame.
        let mut lang_versions = std::collections::BTreeMap::new();
        for lang in ["node", "python", "go", "java"] {
            let version = fine_ill_do_it_myself::lang::active_lang_version(&env, lang)
                .unwrap_or_else(|| "not installed".to_string());
            lang_versions.insert(lang.to_string(), version);
        }

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
        for (key, value) in [
            ("http", ports.http),
            ("https", ports.https),
            ("fpm_base", ports.fpm_base),
            ("mariadb", ports.mariadb),
        ] {
            rows.push(ConfigRow::Port {
                key: key.into(),
                value,
            });
        }

        RefreshData {
            services: statuses,
            language_reps,
            ports,
            php_row,
            lang_versions,
            config_rows: rows,
        }
    }
}

fn apply_refresh(app: &mut App, data: RefreshData) {
    app.services = data.services;
    app.language_reps = data.language_reps;
    app.ports = data.ports;
    app.php_row = data.php_row;
    app.lang_versions = data.lang_versions;
    app.config_rows = data.config_rows;
    app.data_revision += 1;
    app.clamp_now();
    app.normalize_left_cursor();
}

fn start_refresh(
    env: &fine_ill_do_it_myself::Env,
    app: &mut App,
    tx: std::sync::mpsc::Sender<WorkerMsg>,
) {
    if app.refreshing {
        return;
    }
    app.refreshing = true;
    let refresh = make_refresh(env.clone());
    let spawned = std::thread::Builder::new()
        .name("fidim-refresh".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(refresh))
                .map_err(|_| "refresh failed unexpectedly".to_string());
            let _ = tx.send(WorkerMsg::RefreshDone(result));
        });
    if let Err(e) = spawned {
        app.refreshing = false;
        app.message = format!("cannot start refresh worker: {e}");
    }
}

/// Recompute the cached management panel off the UI thread.
fn start_mgmt_refresh(
    env: &fine_ill_do_it_myself::Env,
    app: &mut App,
    tx: std::sync::mpsc::Sender<WorkerMsg>,
) {
    if app.detail.is_none() {
        app.mgmt_for = None;
        app.mgmt_pending = None;
        return;
    }
    let key = app.mgmt_key();
    if app.mgmt_for == Some(key) || app.mgmt_pending.is_some() {
        return;
    }
    app.mgmt_pending = Some(key);
    let env = env.clone();
    let services = app.services.clone();
    let cursor_left = app.cursor_left;
    let config_rows = app.config_rows.clone();
    let spawned = std::thread::Builder::new()
        .name("fidim-management".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mgmt_panel_for(&env, &services, cursor_left, &config_rows)
            }))
            .map_err(|_| "management panel refresh failed unexpectedly".to_string());
            let _ = tx.send(WorkerMsg::ManagementDone { key, result });
        });
    if let Err(e) = spawned {
        app.mgmt_pending = None;
        app.message = format!("cannot start management worker: {e}");
    }
}

fn apply_mgmt_refresh(
    app: &mut App,
    key: (usize, bool, u64),
    result: Result<(String, Vec<String>), String>,
) {
    if app.mgmt_pending == Some(key) {
        app.mgmt_pending = None;
    }
    if app.mgmt_key() != key {
        return;
    }
    match result {
        Ok((title, lines)) => {
            app.mgmt_title = title;
            app.mgmt_line_count = lines.len();
            app.mgmt_lines = lines;
            app.mgmt_for = Some(key);
            app.clamp_now();
        }
        Err(error) => {
            app.mgmt_for = Some(key);
            app.message = error;
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
    fn tab_keeps_empty_projects_panel_available_for_quick_create() {
        let mut a = App::new(vec![svc("caddy", false)], vec![]);

        a.on_key(KeyCode::Tab);
        assert_eq!(focused_panel(&a), "projects");

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

    // ---- quick-create flow -------------------------------------------------

    /// Flatten a ratatui buffer into text, one line per terminal row.
    fn rendered_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn modal_defaults_to_the_offline_php_template() {
        let mut app = App::new(vec![], vec![]);

        app.begin_quick_create();

        assert_eq!(app.quick_create, Some(QuickCreateField::Name));
        assert_eq!(
            fine_ill_do_it_myself::project::Template::ALL[app.template_cursor],
            fine_ill_do_it_myself::project::Template::Php
        );
    }

    #[test]
    fn footer_hints_name_the_new_project_key_only_on_the_projects_panel() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["blog".into()]);
        let services = footer_hints(&a);
        assert!(
            !services.iter().any(|(key, _)| key == "n"),
            "services panel must not claim n: {services:?}"
        );

        a.on_key(KeyCode::Tab); // -> projects (no language rows here)
        let projects = footer_hints(&a);
        assert!(
            projects
                .iter()
                .any(|(key, label)| key == "n" && *label == "new project"),
            "projects panel must offer n: {projects:?}"
        );
    }

    #[test]
    fn new_project_modal_shows_every_field_on_one_screen() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = App::new(vec![], vec![]);
        app.begin_quick_create();
        app.name_input = "blog".into();
        app.with_db = true;
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();

        term.draw(|f| {
            let area = f.area();
            render_quick_create(&app, f, area);
        })
        .unwrap();

        let text = rendered_text(term.backend().buffer());
        for needle in [
            "New project",
            "Name",
            "blog",
            "https://blog.test",
            "Template",
            "Laravel",
            "WordPress",
            "Python",
            "[x] Create a MariaDB database",
            "Enter: create",
            "Esc: cancel",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn quick_create_modal_renders_over_the_side_container() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        app.detail = Some("detail: caddy".into());
        app.panel_right = true;
        app.mgmt_lines = vec!["web.use nginx".into(), "  version 1.2.3".into()];
        app.quick_create = Some(QuickCreateField::Name);
        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();

        term.draw(|f| render(&app, f, f.area())).unwrap();

        let buffer = term.backend().buffer();
        assert_eq!(
            buffer[(78, 2)].symbol(),
            "─",
            "the modal's top border must not be overwritten by side-panel content"
        );
        assert_eq!(
            buffer[(99, 2)].symbol(),
            "╮",
            "the modal's top-right border must be drawn over the side panel"
        );
        let text = rendered_text(buffer);
        assert!(
            text.contains("New project"),
            "the new-project modal must remain visible:\n{text}"
        );
    }

    #[test]
    fn rendered_selected_httpd_panel_contains_httpd_install_action() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let mut app = App::new(vec![svc("httpd", false)], vec![]);
        app.detail = Some("detail: httpd".into());
        app.panel_right = true;
        let (_, lines) = mgmt_panel_for(&env, &app.services, 0, &[]);
        app.mgmt_lines = lines;
        app.mgmt_title = " httpd management ".into();

        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();
        term.draw(|f| render(&app, f, f.area())).unwrap();

        assert!(
            rendered_text(term.backend().buffer()).contains("httpd.install"),
            "the rendered management panel must show httpd.install"
        );
    }

    #[test]
    fn template_picker_options_show_all_templates_with_useful_descriptions() {
        let options = quick_create_template_options();

        assert_eq!(
            options.len(),
            fine_ill_do_it_myself::project::Template::ALL.len()
        );
        assert!(options
            .iter()
            .all(|(_, description)| !description.is_empty()));
        assert!(options.iter().any(|(template, description)| {
            *template == fine_ill_do_it_myself::project::Template::Laravel
                && description.contains("Composer")
        }));
        assert!(options.iter().any(|(template, description)| {
            *template == fine_ill_do_it_myself::project::Template::Node
                && description.contains("3000")
        }));
    }

    #[test]
    fn n_starts_quick_create_from_projects_panel() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        a.on_key(KeyCode::Tab); // -> projects
        assert_eq!(focused_panel(&a), "projects");
        a.on_key(KeyCode::Char('n'));
        assert!(a.quick_create.is_some(), "n must start the flow");
        assert_eq!(a.quick_create.unwrap(), QuickCreateField::Name);
        assert!(a.name_input.is_empty());
    }

    #[test]
    fn quick_create_can_start_when_no_projects_exist_yet() {
        let mut a = App::new(vec![svc("caddy", false)], vec![]);

        a.on_key(KeyCode::Tab);

        assert_eq!(focused_panel(&a), "projects");
        a.on_key(KeyCode::Char('n'));
        assert_eq!(a.quick_create, Some(QuickCreateField::Name));
    }

    #[test]
    fn enter_in_the_name_field_creates_with_the_default_php_template() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "blog".chars() {
            a.on_key(KeyCode::Char(c));
        }

        a.on_key(KeyCode::Enter);

        assert!(a.quick_create.is_none(), "Enter must submit the form");
        assert_eq!(
            a.actions,
            vec![Action::CreateProject {
                name: "blog".into(),
                template: fine_ill_do_it_myself::project::Template::Php,
                db: false,
            }]
        );
    }

    #[test]
    fn an_empty_name_never_queues_a_project() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));

        a.on_key(KeyCode::Enter);
        a.on_key(KeyCode::Enter);

        assert!(a.quick_create.is_some(), "the form must stay open");
        assert!(
            a.actions.is_empty(),
            "an empty name must not create anything"
        );
    }

    #[test]
    fn tab_walks_name_template_and_database_fields() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "blog".chars() {
            a.on_key(KeyCode::Char(c));
        }

        a.on_key(KeyCode::Tab); // template
        a.on_key(KeyCode::Tab); // database
        a.on_key(KeyCode::Char(' '));

        assert!(a.with_db, "Space in the database field must tick it");
        a.on_key(KeyCode::Tab); // back to name
        a.on_key(KeyCode::Backspace);
        assert_eq!(a.name_input, "blo", "the name field must be editable again");
    }

    #[test]
    fn enter_from_the_database_field_creates_with_the_database() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "shop".chars() {
            a.on_key(KeyCode::Char(c));
        }
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char(' '));

        a.on_key(KeyCode::Enter);

        assert_eq!(
            a.actions,
            vec![Action::CreateProject {
                name: "shop".into(),
                template: fine_ill_do_it_myself::project::Template::Php,
                db: true,
            }]
        );
    }

    #[test]
    fn quick_create_action_creates_project_and_refreshes_the_list() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        let runner = SpawnRecorder::default();
        let mut app = App::new(vec![], vec![]);

        perform_action(
            &env,
            runner,
            &mut app,
            &Action::CreateProject {
                name: "landing".into(),
                template: fine_ill_do_it_myself::project::Template::Html,
                db: false,
            },
        );

        assert!(env.paths.sites().join("landing/index.html").is_file());
        assert_eq!(app.projects, vec!["landing"]);
        assert!(app.message.contains("created landing"), "{}", app.message);
    }

    #[test]
    fn n_ignored_outside_projects_panel() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        a.on_key(KeyCode::Char('n')); // services panel
        assert!(a.quick_create.is_none());
    }

    #[test]
    fn new_project_form_keeps_name_and_template_together() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab); // into projects
        a.on_key(KeyCode::Char('n'));
        for c in "my-site".chars() {
            a.on_key(KeyCode::Char(c));
        }
        assert_eq!(a.name_input, "my-site");
        a.on_key(KeyCode::Tab);
        assert_eq!(a.quick_create, Some(QuickCreateField::Template));
        assert_eq!(a.name_input, "my-site");
        assert_eq!(
            fine_ill_do_it_myself::project::Template::ALL[a.template_cursor],
            fine_ill_do_it_myself::project::Template::Php,
            "name and template live in the same form"
        );
    }

    #[test]
    fn project_form_tabs_between_fields_and_keeps_database_selection() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "my-site".chars() {
            a.on_key(KeyCode::Char(c));
        }

        a.on_key(KeyCode::Tab);
        assert_eq!(a.quick_create, Some(QuickCreateField::Template));
        a.on_key(KeyCode::Char('d'));
        a.on_key(KeyCode::BackTab);
        assert_eq!(a.quick_create, Some(QuickCreateField::Name));
        assert!(a.with_db);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Enter);

        assert_eq!(
            a.actions,
            vec![Action::CreateProject {
                name: "my-site".into(),
                template: fine_ill_do_it_myself::project::Template::Php,
                db: true,
            }]
        );
    }

    #[test]
    fn the_form_rejects_an_empty_or_invalid_name_and_shows_the_reason() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        a.on_key(KeyCode::Enter);
        assert!(a.quick_create.is_some(), "empty name keeps the form open");
        assert!(a.form_error.is_some(), "the reason must be shown inline");

        a.on_key(KeyCode::Char('b'));
        a.on_key(KeyCode::Char('a'));
        a.on_key(KeyCode::Char('d'));
        a.on_key(KeyCode::Char('-'));
        a.on_key(KeyCode::Enter);
        assert!(a.quick_create.is_some(), "a trailing - keeps the form open");
        assert!(a.actions.is_empty());
    }

    #[test]
    fn esc_cancels_the_form_without_creating_anything() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "my-site".chars() {
            a.on_key(KeyCode::Char(c));
        }
        a.on_key(KeyCode::Esc);

        assert!(a.quick_create.is_none());
        assert!(a.name_input.is_empty());
        assert!(a.actions.is_empty());
    }
    #[test]
    fn template_field_arrows_move_the_selection_and_enter_creates() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "my-site".chars() {
            a.on_key(KeyCode::Char(c));
        }
        a.on_key(KeyCode::Tab); // template field
        a.on_key(KeyCode::Right);
        a.on_key(KeyCode::Right);
        assert_eq!(
            fine_ill_do_it_myself::project::Template::ALL[a.template_cursor],
            fine_ill_do_it_myself::project::Template::Blank
        );
        a.on_key(KeyCode::Char('d')); // toggle db
        assert!(a.with_db);
        a.on_key(KeyCode::Enter);
        assert!(a.quick_create.is_none());
        assert_eq!(
            a.actions,
            vec![Action::CreateProject {
                name: "my-site".into(),
                template: fine_ill_do_it_myself::project::Template::Blank,
                db: true,
            }]
        );
    }

    #[test]
    fn template_navigation_clamps_at_both_ends() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));
        for c in "my-site".chars() {
            a.on_key(KeyCode::Char(c));
        }
        a.on_key(KeyCode::Tab); // template field
        for _ in 0..10 {
            a.on_key(KeyCode::Up);
        }
        assert_eq!(a.template_cursor, 0, "k never wraps past the top");
        let last = fine_ill_do_it_myself::project::Template::ALL.len() - 1;
        for _ in 0..10 {
            a.on_key(KeyCode::Char('j'));
        }
        assert_eq!(a.template_cursor, last, "j clamps at the last template");
    }

    #[test]
    fn jk_type_into_the_name_instead_of_moving_the_template() {
        let mut a = App::new(vec![svc("caddy", false)], vec!["x".into()]);
        a.on_key(KeyCode::Tab);
        a.on_key(KeyCode::Char('n'));

        for c in "jak".chars() {
            a.on_key(KeyCode::Char(c));
        }

        assert_eq!(a.name_input, "jak");
        assert_eq!(
            fine_ill_do_it_myself::project::Template::ALL[a.template_cursor],
            fine_ill_do_it_myself::project::Template::Php
        );
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
        assert_eq!(ports_label("php-8.4", "8.4", &ports), "8.4");
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
        for want in ["d", "enter", "tab", "j", "k", "o"] {
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
    fn management_enter_ignores_the_displayed_web_version() {
        let mut app = App::new(vec![], vec![]);

        app.mgmt_enter(&["web.use caddy  [2.10.2]".to_string()]);

        assert_eq!(
            app.actions,
            vec![Action::UseWeb("caddy".into())],
            "version text is display-only and must not become the server name"
        );
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
    fn web_management_panel_offers_manifest_backed_install_for_uninstalled_server() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("caddy", false)], 0, &[]);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("caddy.install 2.10.2")),
            "uninstalled manifest-backed web server should have an install action: {lines:?}"
        );
    }

    #[test]
    fn web_management_panel_only_lists_selected_web_server_install_action() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("caddy", false)], 0, &[]);

        assert!(
            lines.iter().any(|line| line.contains("caddy.install")),
            "selected server must expose its installer: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("httpd.install")),
            "unselected server installers must not be mixed into this panel: {lines:?}"
        );
    }

    #[test]
    fn web_management_panel_offers_httpd_install_when_httpd_is_selected() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("httpd", false)], 0, &[]);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("httpd.install 2.4.67")),
            "selected httpd must expose its installer: {lines:?}"
        );
    }

    #[test]
    fn web_management_panel_offers_apache_install_and_hides_unavailable_installers() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, apache) = mgmt_panel_for(&env, &[svc("httpd", false)], 0, &[]);
        let (_, nginx) = mgmt_panel_for(&env, &[svc("nginx", false)], 0, &[]);
        let (_, lighttpd) = mgmt_panel_for(&env, &[svc("lighttpd", false)], 0, &[]);

        assert!(apache
            .iter()
            .any(|line| line.contains("httpd.install 2.4.67")));
        assert!(nginx
            .iter()
            .any(|line| line.contains("nginx.install 1.31.6")));
        assert!(lighttpd
            .iter()
            .any(|line| line.contains("lighttpd.install 1.4.83")));
    }

    #[test]
    fn web_management_panel_uses_local_manifest_entries() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        std::fs::write(
            env.manifest_path(),
            r#"
[[binaries]]
name = "nginx"
version = "1.29.2"
url = "https://example.test/nginx.tar.gz"
"#,
        )
        .unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("nginx", false)], 0, &[]);

        assert!(lines
            .iter()
            .any(|line| line.contains("nginx.install 1.29.2")));
    }

    #[test]
    fn web_management_panel_offers_httpd_install_when_only_its_directory_exists() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        std::fs::create_dir_all(env.paths.bin().join("httpd")).unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("httpd", false)], 0, &[]);

        assert!(lines.iter().any(|line| line.contains("httpd.install")));
    }

    #[test]
    fn global_management_panel_offers_uninstalled_httpd() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(
            &env,
            &[],
            0,
            &[ConfigRow::UseWeb {
                name: "httpd".into(),
                ver: None,
            }],
        );

        assert!(lines
            .iter()
            .any(|line| line.contains("httpd.install 2.4.67")));
    }

    #[test]
    fn global_management_panel_offers_httpd_when_runtime_directory_exists() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        std::fs::create_dir_all(env.paths.bin().join("httpd")).unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[], 0, &[]);

        assert!(lines
            .iter()
            .any(|line| line.contains("httpd.install 2.4.67")));
    }

    #[test]
    fn bundled_manifest_includes_nginx_and_lighttpd_install_sources() {
        let manifest =
            fine_ill_do_it_myself::Manifest::parse(fine_ill_do_it_myself::installer_defaults())
                .unwrap();

        for server in ["nginx", "lighttpd"] {
            assert!(
                manifest.find(server).is_some(),
                "missing bundled source for {server}"
            );
        }
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
        let project_dir = "~/.local/share/fine-ill-do-it-myself/projects";
        let plan = quick_create_plan(project_dir, "shop");
        assert_eq!(
            plan,
            vec![
                "mkdir -p ~/.local/share/fine-ill-do-it-myself/projects/shop",
                "127.0.0.1 shop.test"
            ]
        );
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
    fn d_on_node_opens_node_management_not_php_versions() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let mut lock = fine_ill_do_it_myself::Lock::default();
        lock.languages.insert("node".into(), vec!["24".to_string()]);
        lock.language_defaults
            .insert("node".into(), "24".to_string());
        lock.save(&env.lock_path()).unwrap();

        let services = vec![
            svc("caddy", true),
            svc("php-8.4", false),
            svc("node-24", false),
        ];
        let mut app = App::new(services.clone(), vec![]);
        app.language_reps = language_representatives(&env, &app.services);
        app.cursor_left = *app
            .language_reps
            .iter()
            .find(|&&idx| language_family(&app.services[idx].name) == "node")
            .unwrap();

        let started = std::time::Instant::now();
        handle_key(&mut app, KeyCode::Char('d'));
        let (title, _) = mgmt_panel_for(&env, &app.services, app.cursor_left, &app.config_rows);

        assert!(
            started.elapsed() < Duration::from_millis(500),
            "opening Node management blocked for {:?}",
            started.elapsed()
        );
        assert_eq!(title, " node versions (current: 24) ");
        assert_eq!(app.detail.as_deref(), Some("detail: node-24"));
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
    fn enter_on_php_row_opens_version_list_without_toggling_service() {
        let mut a = App::new(vec![svc("php-8.4", true)], vec![]);
        assert_eq!(focused_panel(&a), "languages");

        a.on_key(KeyCode::Enter);

        assert!(a.actions.is_empty());
        assert!(a.panel_right);
        assert_eq!(a.detail.as_deref(), Some("detail: php-8.4"));
        assert_eq!(a.message, "select a version, Enter to activate");
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

    // ---- composer management row ------------------------------------------

    #[test]
    fn mgmt_panel_lists_composer_with_its_install_action() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let app = App::new(vec![], vec!["blog".into()]);

        let (_, lines) = mgmt_panel_for(&env, &app.services, app.cursor_left, &app.config_rows);

        assert!(
            lines.iter().any(|l| l.contains("composer.install")),
            "composer must be installable from the panel: {lines:?}"
        );
        for component in ["php.install", "mariadb.install", "phpmyadmin.install"] {
            assert!(
                lines.iter().any(|line| line.contains(component)),
                "{component} must be installable from the panel: {lines:?}"
            );
        }
        assert!(
            lines.iter().any(|line| line.contains("node.install 24")),
            "language runtimes must be installable from the panel: {lines:?}"
        );
    }

    #[test]
    fn php_management_panel_offers_uninstalled_versions() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("php", false)], 0, &[]);

        assert!(
            lines.iter().any(|line| line.contains("php.install 8.4")),
            "PHP install options should be shown alongside installed versions: {lines:?}"
        );
    }

    #[test]
    fn php_management_panel_does_not_offer_database_components() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("php", false)], 0, &[]);

        for component in ["mariadb.install", "phpmyadmin.install"] {
            assert!(
                !lines.iter().any(|line| line.contains(component)),
                "the PHP panel must not offer database components ({component}): {lines:?}"
            );
        }
    }

    #[test]
    fn managed_install_rows_hide_mariadb_when_a_runtime_is_available() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let mariadbd = env.paths.runtimes().join("mariadb/11.4.13/bin/mariadbd");
        std::fs::create_dir_all(mariadbd.parent().unwrap()).unwrap();
        std::fs::write(&mariadbd, "binary").unwrap();

        let rows = managed_install_rows(&env);

        assert!(
            !rows.iter().any(|row| row.contains("mariadb.install")),
            "an installed managed runtime must not be offered for installation: {rows:?}"
        );
    }

    #[test]
    fn language_management_panel_offers_uninstalled_versions() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();

        let (_, lines) = mgmt_panel_for(&env, &[svc("node", false)], 0, &[]);

        assert!(
            lines.iter().any(|line| line.contains("node.install 24")),
            "Node.js install options should be shown in its management panel: {lines:?}"
        );
    }

    #[test]
    fn mgmt_panel_marks_an_installed_composer() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        std::fs::write(env.paths.bin().join("composer"), "phar").unwrap();
        let app = App::new(vec![], vec!["blog".into()]);

        let (_, lines) = mgmt_panel_for(&env, &app.services, app.cursor_left, &app.config_rows);

        assert!(
            lines
                .iter()
                .any(|l| l.contains("composer") && l.contains("2.10.3") && l.contains("installed")),
            "installed composer must show its pinned version: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("composer.install")),
            "no install action once it is there: {lines:?}"
        );
    }

    #[test]
    fn enter_on_the_composer_row_queues_the_install_action() {
        let mut a = App::new(vec![], vec![]);

        a.mgmt_enter(&["  composer.install 2.10.3".to_string()]);

        assert_eq!(a.actions, vec![Action::InstallComposer]);
    }

    #[test]
    fn enter_on_install_rows_queues_runtime_and_component_actions() {
        let cases = [
            ("  php.install 8.4", Action::InstallPhp("8.4".into())),
            ("  caddy.install 2.10.2", Action::InstallWeb("caddy".into())),
            ("  httpd.install 2.4.67", Action::InstallWeb("httpd".into())),
            (
                "  node.install 24",
                Action::InstallLang {
                    lang: "node".into(),
                    version: "24".into(),
                },
            ),
            (
                "  mariadb.install 11.4.13",
                Action::InstallManaged("mariadb".into()),
            ),
            (
                "  phpmyadmin.install 5.2.3",
                Action::InstallManaged("phpmyadmin".into()),
            ),
        ];

        for (row, expected) in cases {
            let mut app = App::new(vec![], vec![]);
            app.mgmt_enter(&[row.into()]);
            assert_eq!(app.actions, vec![expected], "row {row}");
        }
    }

    // ---- `o` opens the project in the code editor -------------------------

    #[derive(Clone, Default)]
    struct SpawnRecorder {
        spawned: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl fine_ill_do_it_myself::Runner for SpawnRecorder {
        fn spawn(&self, cmd: &[String]) -> std::result::Result<u32, std::io::Error> {
            self.spawned.lock().unwrap().push(cmd.to_vec());
            Ok(1)
        }
        fn is_running(&self, _pid: u32) -> bool {
            false
        }
        fn kill(&self, _pid: u32) -> std::result::Result<(), std::io::Error> {
            Ok(())
        }
    }

    fn editor_env(t: &tempfile::TempDir) -> (fine_ill_do_it_myself::Env, std::path::PathBuf) {
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let site = env.paths.sites().join("blog");
        std::fs::create_dir_all(&site).unwrap();
        (env, site)
    }

    fn gui_editor() -> fine_ill_do_it_myself::editor::Editor {
        fine_ill_do_it_myself::editor::Editor {
            program: std::path::PathBuf::from("/usr/bin/code"),
            args: Vec::new(),
            kind: fine_ill_do_it_myself::editor::EditorKind::Gui,
        }
    }

    #[test]
    fn o_on_a_project_queues_the_editor_action() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        a.on_key(KeyCode::Tab); // services -> projects (no languages installed)
        assert_eq!(focused_panel(&a), "projects");

        a.on_key(KeyCode::Char('o'));

        assert_eq!(a.actions, vec![Action::OpenEditor("blog".into())]);
    }

    #[test]
    fn o_outside_the_projects_container_only_hints() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        assert_eq!(focused_panel(&a), "services");

        a.on_key(KeyCode::Char('o'));

        assert!(a.actions.is_empty(), "must not open anything from services");
        assert!(
            a.message.contains("project"),
            "message should tell the user to pick a project, got {:?}",
            a.message
        );
    }

    #[test]
    fn editor_keybind_can_be_remapped() {
        let mut a = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        a.keybinds = load_keybinds("editor = e\n");
        a.on_key(KeyCode::Tab);

        a.on_key(KeyCode::Char('e'));
        assert_eq!(a.actions, vec![Action::OpenEditor("blog".into())]);

        a.actions.clear();
        a.on_key(KeyCode::Char('o'));
        assert!(a.actions.is_empty(), "the default key must stop working");
    }

    #[test]
    fn open_editor_spawns_a_gui_editor_detached() {
        let t = tempfile::tempdir().unwrap();
        let (env, site) = editor_env(&t);
        let runner = SpawnRecorder::default();
        let mut app = App::new(vec![], vec!["blog".into()]);
        app.editor = Some(gui_editor());

        perform_action(
            &env,
            runner.clone(),
            &mut app,
            &Action::OpenEditor("blog".into()),
        );

        let spawned = runner.spawned.lock().unwrap().clone();
        assert_eq!(
            spawned,
            vec![vec![
                "/usr/bin/code".to_string(),
                site.display().to_string()
            ]]
        );
        assert!(app.pending_terminal_editor.is_none());
        assert!(app.message.contains("blog"), "got {:?}", app.message);
    }

    #[test]
    fn open_editor_defers_a_terminal_editor_to_the_event_loop() {
        let t = tempfile::tempdir().unwrap();
        let (env, site) = editor_env(&t);
        let runner = SpawnRecorder::default();
        let mut app = App::new(vec![], vec!["blog".into()]);
        app.editor = Some(fine_ill_do_it_myself::editor::Editor {
            program: std::path::PathBuf::from("/usr/bin/vim"),
            args: Vec::new(),
            kind: fine_ill_do_it_myself::editor::EditorKind::Terminal,
        });

        perform_action(
            &env,
            runner.clone(),
            &mut app,
            &Action::OpenEditor("blog".into()),
        );

        assert!(
            runner.spawned.lock().unwrap().is_empty(),
            "a terminal editor must not be spawned behind the TUI"
        );
        assert_eq!(
            app.pending_terminal_editor,
            Some(vec!["/usr/bin/vim".to_string(), site.display().to_string()])
        );
    }

    #[test]
    fn open_editor_without_a_detected_editor_explains_how_to_set_one() {
        let t = tempfile::tempdir().unwrap();
        let (env, _site) = editor_env(&t);
        let runner = SpawnRecorder::default();
        let mut app = App::new(vec![], vec!["blog".into()]);
        app.editor = None;

        perform_action(
            &env,
            runner.clone(),
            &mut app,
            &Action::OpenEditor("blog".into()),
        );

        assert!(runner.spawned.lock().unwrap().is_empty());
        assert!(
            app.message.contains("FIDIM_EDITOR"),
            "message should name the override variable, got {:?}",
            app.message
        );
    }

    // ---- worker thread, progress and cancellation --------------------------

    #[test]
    fn queued_actions_are_taken_first_in_first_out() {
        let mut queue = vec![Action::RestartAll, Action::StopService(0)];

        assert_eq!(take_next(&mut queue), Some(Action::RestartAll));
        assert_eq!(take_next(&mut queue), Some(Action::StopService(0)));
        assert_eq!(take_next(&mut queue), None);
    }

    #[test]
    fn a_worker_job_creates_the_project_and_reports_back() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        let mut app = App::new(vec![], vec![]);
        let (tx, rx) = std::sync::mpsc::channel();

        start_job(
            &env,
            &mut app,
            Action::CreateProject {
                name: "landing".into(),
                template: fine_ill_do_it_myself::project::Template::Html,
                db: false,
            },
            tx,
        );

        assert!(app.job.is_some(), "the job must be visible while it runs");
        let outcome = loop {
            match rx.recv_timeout(Duration::from_secs(20)).unwrap() {
                WorkerMsg::Done(outcome) => break outcome,
                WorkerMsg::Progress { .. } => {}
                WorkerMsg::RefreshDone(_) | WorkerMsg::ManagementDone { .. } => {}
            }
        };
        apply_outcome(&mut app, outcome);
        app.job = None;

        assert!(env.paths.sites().join("landing/index.html").is_file());
        assert_eq!(app.projects, vec!["landing"]);
        assert!(app.message.contains("created landing"), "{}", app.message);
    }

    #[test]
    fn esc_cancels_a_running_job_instead_of_quitting() {
        let mut a = App::new(vec![], vec![]);
        a.job = Some(Job::new("downloading composer"));

        a.on_key(KeyCode::Esc);

        let job = a.job.as_ref().expect("the job must still be tracked");
        assert!(job.cancel.is_cancelled(), "Esc must ask the worker to stop");
        assert!(!a.quit, "Esc must not quit while work is running");
        assert!(a.message.contains("cancelling"), "{}", a.message);
    }

    #[test]
    fn footer_hints_offer_cancel_while_a_job_runs() {
        let mut a = App::new(vec![], vec![]);
        a.job = Some(Job::new("starting caddy"));

        let hints = footer_hints(&a);

        assert!(
            hints
                .iter()
                .any(|(key, label)| key == "esc" && *label == "cancel"),
            "{hints:?}"
        );
        assert!(
            hints.iter().any(|(key, _)| key == "q"),
            "quit must stay reachable: {hints:?}"
        );
    }

    #[test]
    fn spinner_advances_with_time() {
        assert_eq!(spinner(Duration::from_millis(0)), "⠋");
        assert_eq!(spinner(Duration::from_millis(250)), "⠙");
        assert_eq!(spinner(Duration::from_millis(1000)), "⠋");
    }

    #[test]
    fn applying_a_detail_outcome_opens_the_cached_panel() {
        let mut app = App::new(vec![svc("caddy", true)], vec![]);

        apply_outcome(
            &mut app,
            Outcome {
                message: "ok".into(),
                detail: Some("Service: caddy".into()),
                created_project: None,
                terminal_editor: None,
            },
        );

        assert_eq!(app.detail.as_deref(), Some("Service: caddy"));
        assert!(app.panel_right, "the detail panel must take focus");
        assert_eq!(app.cursor_right, 0);
    }

    // ---- renderer ---------------------------------------------------------

    #[test]
    fn render_shows_services_progress_and_cancel_hint() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = App::new(
            vec![svc("caddy", true), svc("php-8.4", false)],
            vec!["blog".into()],
        );
        app.php_row = "8.4".into();
        app.lang_versions.insert("node".into(), "24".into());
        app.job = Some(Job::new("downloading composer"));
        app.progress = Some((50, 100));
        app.message = "working".into();
        let mut term = Terminal::new(TestBackend::new(140, 24)).unwrap();

        term.draw(|f| {
            let area = f.area();
            render(&app, f, area);
        })
        .unwrap();

        let text = rendered_text(term.backend().buffer());
        for needle in [
            "caddy",
            "php",
            "blog",
            "downloading composer",
            "50%",
            "cancel",
            "working",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        assert!(
            !text.contains("▸"),
            "project rows should not contain a triangle marker:\n{text}"
        );
    }

    #[test]
    fn render_uses_the_cached_management_panel() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = App::new(vec![svc("caddy", true)], vec![]);
        app.detail = Some("detail: caddy".into());
        app.panel_right = true;
        app.mgmt_title = " caddy management ".into();
        app.mgmt_lines = vec![
            "web.use caddy".to_string(),
            "  port http = 8080".to_string(),
        ];
        app.mgmt_line_count = 2;
        app.mgmt_for = Some(app.mgmt_key());
        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();

        term.draw(|f| {
            let area = f.area();
            render(&app, f, area);
        })
        .unwrap();

        let text = rendered_text(term.backend().buffer());
        assert!(text.contains("caddy management"), "{text}");
        assert!(text.contains("port http = 8080"), "{text}");
    }

    #[test]
    fn render_survives_a_tiny_terminal() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let app = App::new(vec![svc("caddy", true)], vec!["blog".into()]);
        let mut term = Terminal::new(TestBackend::new(16, 5)).unwrap();

        term.draw(|f| {
            let area = f.area();
            render(&app, f, area);
        })
        .unwrap();
    }

    #[test]
    fn project_panel_header_shows_the_new_project_shortcut() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = App::new(
            vec![svc("php-8.4", false), svc("node-24", false)],
            vec!["blog".into()],
        );
        app.cursor_left = 2;
        app.php_row = "8.4".into();
        app.lang_versions.insert("node".into(), "24.1.0".into());
        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();

        term.draw(|f| render(&app, f, f.area())).unwrap();

        let text = rendered_text(term.backend().buffer());
        assert!(
            text.contains("projects (n: new project)"),
            "projects header should show the new-project shortcut:\n{text}"
        );
        assert!(
            text.contains("programming languages"),
            "language panel should still be rendered:\n{text}"
        );
        assert!(
            !text.contains("programming languages (n: new project)"),
            "language header must not show the project shortcut:\n{text}"
        );
    }

    #[test]
    fn refresh_caches_ports_runtimes_and_the_management_panel() {
        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let mut app = App::new(vec![svc("caddy", true)], vec![]);
        app.detail = Some("detail: caddy".into());
        app.panel_right = true;

        let refresh = make_refresh(env.clone());
        apply_refresh(&mut app, refresh());
        let mgmt_key = app.mgmt_key();
        let mgmt = mgmt_panel_for(&env, &app.services, app.cursor_left, &app.config_rows);
        apply_mgmt_refresh(&mut app, mgmt_key, Ok(mgmt));

        assert!(!app.php_row.is_empty(), "the php row must be resolved once");
        assert_eq!(app.ports.caddy_http, 8080, "ports are cached for render");
        assert!(
            app.lang_versions.contains_key("node"),
            "runtime versions must be cached: {:?}",
            app.lang_versions
        );
        assert!(
            !app.mgmt_lines.is_empty(),
            "refresh must build the management panel"
        );
        assert_eq!(app.mgmt_for, Some(app.mgmt_key()));
    }

    #[test]
    fn initial_refresh_does_not_block_on_a_slow_version_probe() {
        use std::os::unix::fs::PermissionsExt;

        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let caddy_dir = env.paths.bin().join("caddy");
        std::fs::create_dir_all(&caddy_dir).unwrap();
        let caddy = caddy_dir.join("caddy");
        std::fs::write(&caddy, "#!/bin/sh\nsleep 1\necho 'v2.0.0'\n").unwrap();
        std::fs::set_permissions(&caddy, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut app = App::new(vec![], vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        start_refresh(&env, &mut app, tx);

        assert!(
            started.elapsed() < Duration::from_millis(500),
            "initial refresh blocked the UI for {:?}",
            started.elapsed()
        );
        assert!(app.refreshing, "refresh should be in progress");
        let result = rx.recv_timeout(Duration::from_secs(3));
        assert!(
            matches!(result, Ok(WorkerMsg::RefreshDone(Ok(_)))),
            "refresh worker did not return successfully"
        );
    }

    #[test]
    fn management_refresh_does_not_block_on_a_slow_version_probe() {
        use std::os::unix::fs::PermissionsExt;

        let t = tempfile::tempdir().unwrap();
        let env = fine_ill_do_it_myself::Env::new(t.path().join("app"), t.path().join("Sites"));
        env.ensure_dirs().unwrap();
        let caddy_dir = env.paths.bin().join("caddy");
        std::fs::create_dir_all(&caddy_dir).unwrap();
        let caddy = caddy_dir.join("caddy");
        std::fs::write(&caddy, "#!/bin/sh\nsleep 1\necho 'v2.0.0'\n").unwrap();
        std::fs::set_permissions(&caddy, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut app = App::new(vec![svc("caddy", true)], vec![]);
        app.detail = Some("detail: caddy".into());
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        start_mgmt_refresh(&env, &mut app, tx);

        assert!(
            started.elapsed() < Duration::from_millis(500),
            "management refresh blocked the UI for {:?}",
            started.elapsed()
        );
        assert!(app.mgmt_pending.is_some());
        let result = rx.recv_timeout(Duration::from_secs(3));
        assert!(
            matches!(result, Ok(WorkerMsg::ManagementDone { .. })),
            "management worker did not return its result"
        );
    }

    #[test]
    fn php_detail_panel_navigates_through_every_rendered_row() {
        let mut app = App::new(vec![svc("php-8.4", false)], vec![]);
        app.detail = Some("detail: php-8.4".into());
        app.panel_right = true;
        // Only one installed PHP version drives `config_rows`, while the
        // visible panel also lists installable versions and managed components.
        app.config_rows = vec![ConfigRow::UsePhp("8.4".into())];
        app.mgmt_lines = vec![
            "▶ php.use 8.4".into(),
            "  php.install 8.3".into(),
            "  composer.install 2.10.3".into(),
            "  mariadb.install 11.4.13".into(),
        ];
        app.mgmt_line_count = app.mgmt_lines.len();
        app.mgmt_for = Some(app.mgmt_key());

        for expected in 1..app.mgmt_lines.len() {
            app.on_key(KeyCode::Down);
            assert_eq!(
                app.cursor_right, expected,
                "the cursor must reach every rendered PHP panel row"
            );
        }
    }

    #[test]
    fn handle_key_commits_the_typed_port_from_the_cached_lines() {
        let mut a = App::new(vec![svc("nginx", false)], vec![]);
        a.detail = Some("detail: nginx".into());
        a.panel_right = true;
        a.mgmt_lines = vec![
            "web.use nginx".to_string(),
            "  version 1.2".to_string(),
            "  port http = 8082".to_string(),
            "  port https = 8445".to_string(),
        ];
        a.mgmt_line_count = 4;
        a.cursor_right = 2;
        a.mgmt_for = Some(a.mgmt_key());

        handle_key(&mut a, KeyCode::Enter);
        assert_eq!(a.input, "8082", "the editor prefills the cached value");
        for c in ['9', '0', '9', '0'] {
            handle_key(&mut a, KeyCode::Char(c));
        }
        handle_key(&mut a, KeyCode::Enter);

        assert!(
            a.actions.contains(&Action::SetPort {
                key: "nginx_http".into(),
                value: 9090,
            }),
            "Enter must commit the typed value, got {:?}",
            a.actions
        );
    }

    #[test]
    fn enter_does_not_use_a_stale_management_panel() {
        let mut app = App::new(vec![svc("nginx", false)], vec![]);
        app.detail = Some("detail: nginx".into());
        app.panel_right = true;
        app.mgmt_lines = vec!["  port http = 8082".to_string()];

        handle_key(&mut app, KeyCode::Enter);

        assert!(
            app.edit.is_none(),
            "stale panel must not open the port editor"
        );
        assert!(
            app.actions.is_empty(),
            "stale panel must not queue an action"
        );
        assert!(app.message.contains("loading"));
    }
}
