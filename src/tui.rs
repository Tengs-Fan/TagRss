//! The terminal user interface: folders on the left, the selected folder's entries on the right.
//!
//! `App` holds the state and turns keys into `Action`s without touching the core, so it can be
//! tested; `run` is the thin loop that performs actions on the core and draws.

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::sync::mpsc;
use std::time::Duration;

use tagrss::{Core, Entry, FolderSummary, SyncEvent, SyncSummary};

use crate::pad;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Folders,
    Entries,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    /// The folder selection changed; load its entries.
    LoadFolder,
    Open(i64),
    SetRead(i64, bool),
    MarkFolderRead(String),
    Sync,
    Reload,
}

pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub new_entries: usize,
    pub failed: usize,
}

pub struct App {
    pub folders: Vec<FolderSummary>,
    pub entries: Vec<Entry>,
    pub pane: Pane,
    pub status: String,
    pub sync: Option<Progress>,
    folder_state: ListState,
    entry_state: ListState,
}

const HELP: &str = "j/k move · Tab pane · Enter open · r read/unread · R folder read · s sync · c reload · q quit";

impl App {
    pub fn new(folders: Vec<FolderSummary>) -> Self {
        let mut app = App {
            folders: Vec::new(),
            entries: Vec::new(),
            pane: Pane::Folders,
            status: String::new(),
            sync: None,
            folder_state: ListState::default(),
            entry_state: ListState::default(),
        };
        app.set_folders(folders);
        app
    }

    #[cfg(test)]
    pub fn folder_index(&self) -> Option<usize> {
        self.folder_state.selected()
    }

    pub fn selected_folder(&self) -> Option<&FolderSummary> {
        self.folder_state.selected().and_then(|i| self.folders.get(i))
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entry_state.selected().and_then(|i| self.entries.get(i))
    }

    /// Replaces the folder list, keeping the selection where possible.
    pub fn set_folders(&mut self, folders: Vec<FolderSummary>) {
        let keep = self.selected_folder().map(|f| f.name.clone());
        self.folders = folders;
        let index = keep
            .and_then(|name| self.folders.iter().position(|f| f.name == name))
            .or(if self.folders.is_empty() { None } else { Some(0) });
        self.folder_state.select(index);
    }

    /// Replaces the entry list, keeping the cursor on the same entry if it is still there.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let keep = self.selected_entry().map(|e| e.id);
        let old_index = self.entry_state.selected().unwrap_or(0);
        self.entries = entries;
        let index = match keep.and_then(|id| self.entries.iter().position(|e| e.id == id)) {
            Some(i) => Some(i),
            None if self.entries.is_empty() => None,
            None => Some(old_index.min(self.entries.len() - 1)),
        };
        self.entry_state.select(index);
        if self.entries.is_empty() && self.pane == Pane::Entries {
            self.pane = Pane::Folders;
        }
    }

    /// Updates the snapshot in place, so a read entry stays where it is until the list is refreshed.
    pub fn set_read_locally(&mut self, id: i64, read: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.read = read;
        }
    }

    pub fn set_all_read_locally(&mut self) {
        self.entries.iter_mut().for_each(|e| e.read = true);
    }

    fn step(state: &mut ListState, len: usize, down: bool) -> bool {
        let Some(current) = state.selected() else { return false };
        let next = if down { (current + 1).min(len.saturating_sub(1)) } else { current.saturating_sub(1) };
        state.select(Some(next));
        next != current
    }

    pub fn handle(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Tab => {
                self.pane = match self.pane {
                    Pane::Folders if !self.entries.is_empty() => Pane::Entries,
                    _ => Pane::Folders,
                };
                Action::None
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.pane = Pane::Folders;
                Action::None
            }
            KeyCode::Char('l') | KeyCode::Right if !self.entries.is_empty() => {
                self.pane = Pane::Entries;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(false),
            KeyCode::Enter => match self.pane {
                Pane::Folders if !self.entries.is_empty() => {
                    self.pane = Pane::Entries;
                    Action::None
                }
                Pane::Entries => self.selected_entry().map_or(Action::None, |e| Action::Open(e.id)),
                _ => Action::None,
            },
            KeyCode::Char('r') if self.pane == Pane::Entries => {
                self.selected_entry().map_or(Action::None, |e| Action::SetRead(e.id, !e.read))
            }
            KeyCode::Char('R') => self.selected_folder().map_or(Action::None, |f| Action::MarkFolderRead(f.name.clone())),
            KeyCode::Char('s') if self.sync.is_none() => Action::Sync,
            KeyCode::Char('c') => Action::Reload,
            _ => Action::None,
        }
    }

    fn move_cursor(&mut self, down: bool) -> Action {
        match self.pane {
            Pane::Folders => {
                if Self::step(&mut self.folder_state, self.folders.len(), down) {
                    *self.entry_state.offset_mut() = 0;
                    self.entry_state.select(None);
                    Action::LoadFolder
                } else {
                    Action::None
                }
            }
            Pane::Entries => {
                Self::step(&mut self.entry_state, self.entries.len(), down);
                Action::None
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let [main, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        let [left, right] = Layout::horizontal([Constraint::Length(32), Constraint::Min(20)]).areas(main);

        let focused = Style::default().add_modifier(Modifier::REVERSED);
        let unfocused = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().add_modifier(Modifier::DIM);

        let folder_items: Vec<ListItem> = self
            .folders
            .iter()
            .map(|f| {
                let count = if f.unread > 0 { format!("{:>5}", f.unread) } else { "     ".to_string() };
                ListItem::new(Line::from(vec![Span::styled(count, dim), Span::raw("  "), Span::raw(f.name.clone())]))
            })
            .collect();
        let folders = List::new(folder_items)
            .block(Block::bordered().title(" Folders "))
            .highlight_style(if self.pane == Pane::Folders { focused } else { unfocused });
        frame.render_stateful_widget(folders, left, &mut self.folder_state);

        let title = match self.selected_folder() {
            Some(f) => format!(" {} · {} unread of {} ", f.name, f.unread, f.total),
            None => " No folders: add [[folder]] entries to the config file ".to_string(),
        };
        let entry_items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| {
                let marker = if e.read { "  " } else { "● " };
                let line = Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{}  ", e.date.format("%m-%d")), dim),
                    Span::styled(format!("{}  ", pad(&e.sources.join(", "), 20)), dim),
                    Span::raw(e.title.clone()),
                ]);
                ListItem::new(line).style(if e.read { dim } else { Style::default() })
            })
            .collect();
        let entries = List::new(entry_items)
            .block(Block::bordered().title(title))
            .highlight_style(if self.pane == Pane::Entries { focused } else { unfocused });
        frame.render_stateful_widget(entries, right, &mut self.entry_state);

        let text = match &self.sync {
            Some(p) => format!(
                "Syncing {}/{} · {} new · {} failed",
                p.done, p.total, p.new_entries, p.failed
            ),
            None if !self.status.is_empty() => self.status.clone(),
            None => HELP.to_string(),
        };
        frame.render_widget(Paragraph::new(text).style(dim), status);
    }
}

enum SyncMessage {
    Event(SyncEvent),
    Done(Result<SyncSummary>),
}

pub fn run(core: &mut Core) -> Result<()> {
    let mut app = App::new(core.folders()?);
    load_folder(core, &mut app);
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, core, &mut app);
    ratatui::restore();
    result
}

fn load_folder(core: &Core, app: &mut App) {
    let Some(name) = app.selected_folder().map(|f| f.name.clone()) else {
        app.set_entries(Vec::new());
        return;
    };
    match core.entries(&name) {
        Ok(entries) => app.set_entries(entries),
        Err(e) => app.status = format!("{e:#}"),
    }
}

fn refresh_counts(core: &Core, app: &mut App) {
    match core.folders() {
        Ok(folders) => app.set_folders(folders),
        Err(e) => app.status = format!("{e:#}"),
    }
}

fn event_loop(terminal: &mut DefaultTerminal, core: &mut Core, app: &mut App) -> Result<()> {
    let (tx, rx) = mpsc::channel::<SyncMessage>();
    loop {
        terminal.draw(|frame| app.render(frame))?;

        while let Ok(message) = rx.try_recv() {
            match message {
                SyncMessage::Event(event) => {
                    if let Some(p) = app.sync.as_mut() {
                        match event {
                            SyncEvent::Started { sources } => p.total = sources,
                            SyncEvent::Fetched { new_observations, .. } => {
                                p.done += 1;
                                p.new_entries += new_observations;
                            }
                            SyncEvent::Failed { .. } => {
                                p.done += 1;
                                p.failed += 1;
                            }
                        }
                    }
                }
                SyncMessage::Done(result) => {
                    app.sync = None;
                    app.status = match result {
                        Ok(s) => format!("Synced {} sources: {} new, {} failed", s.sources, s.new_observations, s.failed),
                        Err(e) => format!("Sync failed: {e:#}"),
                    };
                    refresh_counts(core, app);
                    load_folder(core, app);
                }
            }
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        app.status.clear();
        match app.handle(key) {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::LoadFolder => load_folder(core, app),
            Action::Open(id) => match core.open_entry(id) {
                Ok(url) => {
                    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                    let spawned = std::process::Command::new(opener)
                        .arg(&url)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                    if let Err(e) = spawned {
                        app.status = format!("cannot run {opener}: {e}");
                    }
                    app.set_read_locally(id, true);
                    refresh_counts(core, app);
                }
                Err(e) => app.status = format!("{e:#}"),
            },
            Action::SetRead(id, read) => match core.mark(&[id], read) {
                Ok(()) => {
                    app.set_read_locally(id, read);
                    refresh_counts(core, app);
                }
                Err(e) => app.status = format!("{e:#}"),
            },
            Action::MarkFolderRead(name) => match core.mark_folder_read(&name) {
                Ok(n) => {
                    app.set_all_read_locally();
                    app.status = format!("Marked {n} entries in \"{name}\" read");
                    refresh_counts(core, app);
                }
                Err(e) => app.status = format!("{e:#}"),
            },
            Action::Sync => {
                app.sync = Some(Progress { done: 0, total: core.config().sources.len(), new_entries: 0, failed: 0 });
                let syncer = core.syncer();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let events = tx.clone();
                    let result = syncer.run(|event| {
                        let _ = events.send(SyncMessage::Event(event.clone()));
                    });
                    let _ = tx.send(SyncMessage::Done(result));
                });
            }
            Action::Reload => match core.reload_config() {
                Ok(warnings) => {
                    app.status = match warnings.first() {
                        None => "Config reloaded".to_string(),
                        Some(w) => format!("Config reloaded with {} warnings; first: {w}", warnings.len()),
                    };
                    refresh_counts(core, app);
                    load_folder(core, app);
                }
                Err(e) => {
                    let first = format!("{e:#}");
                    app.status = format!("Config invalid, kept the old one: {}", first.lines().last().unwrap_or(""));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashSet;

    fn folder(name: &str, unread: usize) -> FolderSummary {
        FolderSummary { name: name.into(), unread, total: unread + 1 }
    }

    fn entry(id: i64, title: &str, read: bool) -> Entry {
        Entry {
            id,
            observation_ids: vec![id],
            title: title.into(),
            url: Some(format!("https://example.com/{id}")),
            date: Utc::now(),
            word_count: None,
            read,
            sources: vec!["Blog".into()],
            tags: HashSet::new(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app() -> App {
        let mut app = App::new(vec![folder("Today", 2), folder("AI", 0)]);
        app.set_entries(vec![entry(10, "First", false), entry(11, "Second", true)]);
        app
    }

    #[test]
    fn moving_between_folders_asks_for_a_load() {
        let mut app = app();
        assert_eq!(app.handle(key(KeyCode::Char('j'))), Action::LoadFolder);
        assert_eq!(app.folder_index(), Some(1));
        assert_eq!(app.handle(key(KeyCode::Char('j'))), Action::None, "already at the bottom");
        assert_eq!(app.handle(key(KeyCode::Up)), Action::LoadFolder);
    }

    #[test]
    fn entry_keys_produce_actions_on_the_selected_entry() {
        let mut app = app();
        assert_eq!(app.handle(key(KeyCode::Char('r'))), Action::None, "r only acts in the entries pane");
        assert_eq!(app.handle(key(KeyCode::Tab)), Action::None);
        assert_eq!(app.pane, Pane::Entries);
        assert_eq!(app.handle(key(KeyCode::Enter)), Action::Open(10));
        assert_eq!(app.handle(key(KeyCode::Char('r'))), Action::SetRead(10, true));
        app.handle(key(KeyCode::Char('j')));
        assert_eq!(app.handle(key(KeyCode::Char('r'))), Action::SetRead(11, false));
        assert_eq!(app.handle(key(KeyCode::Char('R'))), Action::MarkFolderRead("Today".into()));
        assert_eq!(app.handle(key(KeyCode::Char('h'))), Action::None);
        assert_eq!(app.pane, Pane::Folders);
    }

    #[test]
    fn global_keys() {
        let mut app = app();
        assert_eq!(app.handle(key(KeyCode::Char('s'))), Action::Sync);
        app.sync = Some(Progress { done: 0, total: 1, new_entries: 0, failed: 0 });
        assert_eq!(app.handle(key(KeyCode::Char('s'))), Action::None, "no second sync while one runs");
        assert_eq!(app.handle(key(KeyCode::Char('c'))), Action::Reload);
        assert_eq!(app.handle(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(app.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Action::Quit);
    }

    #[test]
    fn refreshing_keeps_the_cursor_on_the_same_entry() {
        let mut app = app();
        app.handle(key(KeyCode::Tab));
        app.handle(key(KeyCode::Char('j')));
        assert_eq!(app.selected_entry().unwrap().id, 11);
        // A sync added a newer entry at the top.
        app.set_entries(vec![entry(12, "New", false), entry(10, "First", false), entry(11, "Second", true)]);
        assert_eq!(app.selected_entry().unwrap().id, 11);
        // The entry disappeared: the cursor stays at the same position.
        app.set_entries(vec![entry(12, "New", false)]);
        assert_eq!(app.selected_entry().unwrap().id, 12);
        app.set_entries(Vec::new());
        assert_eq!(app.pane, Pane::Folders, "an empty list cannot keep focus");
    }

    #[test]
    fn marking_read_keeps_the_entry_in_the_snapshot() {
        let mut app = app();
        app.set_read_locally(10, true);
        assert_eq!(app.entries.len(), 2);
        assert!(app.entries[0].read);
    }

    #[test]
    fn renders_folders_entries_and_status() {
        let mut app = app();
        app.status = "Synced 3 sources".into();
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        for expected in ["Folders", "Today", "AI", "First", "Second", "Today · 2 unread of 3", "Synced 3 sources"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }
}
