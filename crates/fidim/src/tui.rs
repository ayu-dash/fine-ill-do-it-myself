use fine_ill_do_it_myself::{ServiceState, ServiceStatus};

pub struct App {
    pub services: Vec<ServiceStatus>,
    pub projects: Vec<String>,
    pub cursor: usize,
    pub message: String,
    pub quit: bool,
}

impl App {
    pub fn new(services: Vec<ServiceStatus>, projects: Vec<String>) -> Self {
        Self {
            services,
            projects,
            cursor: 0,
            message: String::new(),
            quit: false,
        }
    }

    #[allow(dead_code)]
    pub fn running_count(&self) -> usize {
        self.services
            .iter()
            .filter(|s| s.state == ServiceState::Running)
            .count()
    }

    pub fn on_key(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => {
                self.quit = true
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                self.cursor_move(1)
            }
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                self.cursor_move(-1)
            }
            crossterm::event::KeyCode::Tab => {
                self.cursor = self.cursor.saturating_add(1);
                self.clamp_cursor();
            }
            crossterm::event::KeyCode::Char('r') => self.message = "restart requested".into(),
            _ => {}
        }
    }

    fn cursor_move(&mut self, by: i32) {
        let c = self.cursor as i32 + by;
        self.cursor = if c < 0 { 0 } else { c as usize };
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.services.len().max(self.projects.len()).max(1);
        if self.cursor >= len {
            self.cursor = len - 1;
        }
    }
}

/// Render the app state into a plain multi-line preview (tests), TUI uses
/// the same data layout.
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
    out.push_str("q/esc: quit\n");
    out
}

/// Terminal entry: raw mode + key loop; render pipeline follows in stage 6.
pub fn run_tui(env: &fine_ill_do_it_myself::Env) -> Result<(), fine_ill_do_it_myself::Error> {
    use crossterm::event::{Event, KeyEventKind};
    use crossterm::terminal;
    let _ = terminal::enable_raw_mode();
    let projects: Vec<String> = fine_ill_do_it_myself::list_projects(env);
    let mut app = App::new(Vec::new(), projects);
    loop {
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
    let _ = crossterm::terminal::disable_raw_mode();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn quit_key_sets_flag() {
        let mut a = App::new(vec![], vec![]);
        a.on_key(crossterm::event::KeyCode::Char('q'));
        assert!(a.quit);
    }

    #[test]
    fn arrow_keys_move_cursor_with_clamp() {
        let mut a = App::new(
            vec![svc("caddy", false), svc("php-8.4", false)],
            vec!["blog".into()],
        );
        a.on_key(crossterm::event::KeyCode::Down);
        a.on_key(crossterm::event::KeyCode::Down);
        a.on_key(crossterm::event::KeyCode::Down);
        assert_eq!(a.cursor, 1);
        a.on_key(crossterm::event::KeyCode::Up);
        assert_eq!(a.cursor, 0);
        a.on_key(crossterm::event::KeyCode::Char('k'));
        assert_eq!(a.cursor, 0);
    }

    #[test]
    fn restart_key_sets_message_not_quit() {
        let mut a = App::new(vec![], vec![]);
        a.on_key(crossterm::event::KeyCode::Char('r'));
        assert!(!a.quit);
        assert!(a.message.contains("restart"));
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
}
