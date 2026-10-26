use std::collections::BTreeMap;

use fine_ill_do_it_myself::{ServiceState, ServiceStatus};

#[derive(Clone)]
pub struct Keybinds {
    pub quit: crossterm::event::KeyCode,
    pub restart: crossterm::event::KeyCode,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            quit: crossterm::event::KeyCode::Char('q'),
            restart: crossterm::event::KeyCode::Char('r'),
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

pub struct App {
    pub services: Vec<ServiceStatus>,
    pub projects: Vec<String>,
    pub cursor: usize,
    pub message: String,
    pub quit: bool,
    pub keybinds: Keybinds,
    pub actions: Vec<&'static str>,
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
            _ if key == self.keybinds.restart => {
                self.message = "restart requested".into();
                self.actions.push("restart");
            }
            _ => {}
        }
    }

    fn cursor_move(&mut self, by: i32) {
        let c = self.cursor as i32 + by;
        self.cursor = c.max(0) as usize;
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.services.len().max(self.projects.len()).max(1);
        if self.cursor >= len {
            self.cursor = len - 1;
        }
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

/// Terminal entry: raw mode loop with ratatui rendering of status + logs.
pub fn run_tui(env: &fine_ill_do_it_myself::Env) -> Result<(), fine_ill_do_it_myself::Error> {
    use crossterm::event::{Event, KeyEventKind};
    use crossterm::terminal;
    use ratatui::backend::CrosstermBackend;
    use ratatui::widgets::{Block, BorderType, Paragraph};
    use ratatui::Terminal;
    // refuse to hijack a non-terminal; scripted invocations keep working
    if unsafe { libc::isatty(1) } != 1 {
        println!("TUI requires a tty; use: status");
        return Ok(());
    }
    let _ = terminal::enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), terminal::EnterAlternateScreen);

    let projects: Vec<String> = fine_ill_do_it_myself::list_projects(env);
    let mut app = App::new(Vec::new(), projects);
    let conflines: BTreeMap<(), Vec<String>> = BTreeMap::new();

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend).ok();
    loop {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        if let Some(t) = term.as_mut() {
            let _ = t.draw(|f| {
                use ratatui::layout::{Constraint, Layout, VerticalAlignment};
                use ratatui::text::{Line, Span};
                let [top, bottom] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
                let _ = conflines.get(&());
                let spans = app
                    .services
                    .iter()
                    .map(|s| {
                        let st = match s.state {
                            ServiceState::Running => "RUNNING",
                            _ => "STOPPED",
                        };
                        Span::default().content(format!("{} {}\n", s.name, st))
                    })
                    .collect::<Vec<_>>();
                let mut lines: Vec<Line> = vec![Line::from(
                    "fine-ill-do-it-myself — q: quit, j/k: move, r: restart",
                )];
                for sp in spans {
                    lines.push(Line::from(vec![sp]));
                }
                lines.push(Line::from(
                    app.projects
                        .iter()
                        .map(|p| Span::default().content(format!("  {p}\n")))
                        .collect::<Vec<_>>(),
                ));
                lines.push(Line::from("press q to quit"));
                let _ = Block::default();
                let para = Paragraph::new(lines);
                let _ = BorderType::Rounded;
                f.render_widget(para, top);
                let _ = (bottom, VerticalAlignment::Center);
                f.render_widget(Paragraph::new(Line::from(app.message.as_str())), bottom);
            });
        }
        if crossterm::event::poll(std::time::Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = crossterm::event::read() {
                if k.kind == KeyEventKind::Press {
                    app.on_key(k.code);
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
        assert_eq!(a.cursor, 1);
        a.on_key(KeyCode::Up);
        assert_eq!(a.cursor, 0);
        a.on_key(KeyCode::Char('k'));
        assert_eq!(a.cursor, 0);
    }

    #[test]
    fn restart_key_emits_action() {
        let mut a = App::new(vec![], vec![]);
        a.on_key(KeyCode::Char('r'));
        assert!(!a.quit);
        assert!(a.message.contains("restart"));
        assert_eq!(a.actions, vec!["restart"]);
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
    fn quick_create_plan_covers_dir_and_hosts() {
        let plan = quick_create_plan("~/Sites", "shop");
        assert_eq!(plan, vec!["mkdir -p ~/Sites/shop", "127.0.0.1 shop.test"]);
    }
}
