//! `liftoff tui` — btop/lazygit-style dashboard.
use crate::config::{Config, paths};
use crate::model::{Article, Cache, FlightPlan, Launch, Milestone, clean};
use crate::schedule::{self, State};
use crate::{api, daemon, planner, spacex, store, testflight};
use anyhow::Result;
use chrono::{DateTime, Duration, Local, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::canvas::{Canvas, Map, MapResolution};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Sparkline, Table, TableState, Wrap,
};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime};

enum Msg {
    News(String, Vec<Article>),
    Fetched(Result<Cache, String>),
    Telemetry(Option<(f64, f64)>),
    Patch(String, Option<image::RgbaImage>),
    Plan(String, Option<FlightPlan>),
}

/// Focusable panes, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    Details,
    Plan,
    Telemetry,
    Map,
}

/// What occupies the side slot next to Details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePane {
    Hidden,
    FlightPlan,
    Telemetry,
}

impl SidePane {
    /// Pressing the key for `want` shows it, unless it is already shown (then hide).
    pub fn toggle(self, want: SidePane) -> SidePane {
        if self == want { SidePane::Hidden } else { want }
    }
}

/// Next pane in `order` after `cur` (d = +1 / -1), wrapping. If `cur` is not
/// visible (e.g. Flight Plan hidden) start from the first pane.
pub fn next_focus(order: &[Pane], cur: Pane, d: i64) -> Pane {
    if order.is_empty() {
        return cur;
    }
    match order.iter().position(|p| *p == cur) {
        Some(i) => order[((i as i64 + d).rem_euclid(order.len() as i64)) as usize],
        None => order[0],
    }
}

/// Scroll offset after moving by `d`, clamped to [0, max].
pub fn clamp_scroll(cur: u16, d: i64, max: u16) -> u16 {
    (cur as i64 + d).clamp(0, max as i64) as u16
}

#[derive(PartialEq)]
enum Overlay {
    None,
    Help,
    News,
}

struct App {
    cfg: Config,
    cache: Cache,
    state: State,
    mtime: Option<SystemTime>,
    table: TableState,
    overlay: Overlay,
    news_sel: usize,
    news: std::collections::HashMap<String, Vec<Article>>,
    news_loading: Option<String>,
    telemetry: Vec<(f64, f64)>,
    toast: Option<(String, Instant)>,
    tick: u64,
    list_area: Rect,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    fetching: bool,
    no_color: bool,
    reduced_motion: bool,
    quit: bool,
    patches: std::collections::HashMap<String, Option<image::RgbaImage>>,
    patch_loading: std::collections::HashSet<String>,
    plans: std::collections::HashMap<String, Option<FlightPlan>>,
    plan_loading: std::collections::HashSet<String>,
    details_scroll: u16,
    side: SidePane,
    focus: Pane,
    /// Panes drawn last frame (Tab order) and their areas, for focus + mouse.
    visible: Vec<(Pane, Rect)>,
    details_max: u16,
    /// None = auto-follow the current event; Some = user scrolled.
    plan_scroll: Option<u16>,
    plan_max: u16,
    plan_auto: u16,
    test: Option<TestRun>,
    /// Ctrl-x pressed; followed by 't' 'f' toggles the hidden test flight.
    chord: Option<(Instant, String)>,
    /// Selected launch id: the source of truth for selection across reloads,
    /// re-sorts and test-flight re-injection (the table index is derived).
    sel_id: Option<String>,
    nudge: crate::nudge::Nudge,
    /// LL2 NETs as loaded, so nudges can be re-applied/cleared.
    orig_nets: std::collections::HashMap<String, DateTime<Utc>>,
}

struct TestRun {
    started: Instant,
    prev_t: f64,
    /// Last bar-marker heartbeat write.
    beat: Option<Instant>,
    /// Wall-clock start (ms), to honor a stop issued by another TUI.
    started_ms: i64,
}

// ------------------------------------------------------------------ palette

fn status_color(class: &str) -> Color {
    match class {
        "go" => Color::Rgb(0x8f, 0xd6, 0x7a),
        "tbc" => Color::Rgb(0x7a, 0xc7, 0xd6),
        "hold" => Color::Rgb(0xe8, 0xb9, 0x4a),
        "flight" => Color::Rgb(0xc7, 0x8a, 0xf0),
        "success" => Color::Rgb(0x6f, 0x9b, 0xf2),
        "failure" => Color::Rgb(0xe8, 0x6a, 0x6a),
        _ => Color::Rgb(0x80, 0x86, 0x8f),
    }
}
const ACCENT: Color = Color::Rgb(0xf2, 0x9e, 0x4c);
const DIM: Color = Color::Rgb(0x5c, 0x63, 0x70);
const FG: Color = Color::Rgb(0xd0, 0xd4, 0xdc);
const BORDER: Color = Color::Rgb(0x55, 0x5e, 0x72);
/// Land glyphs: bright enough to read over blurred/transparent backgrounds.
const LAND: Color = Color::Rgb(0x8f, 0xb8, 0xd8);

pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Strip control characters from every displayed string of a cached launch
/// (caches written before the parser cleaned them still contain \r\n).
fn sanitize(l: &mut Launch) {
    for f in [&mut l.name, &mut l.mission, &mut l.mission_description, &mut l.pad, &mut l.location, &mut l.provider, &mut l.rocket, &mut l.status_name] {
        if f.chars().any(|c| c.is_control()) {
            *f = clean(f);
        }
    }
    if let Some(w) = &mut l.weather_concerns {
        *w = clean(w);
    }
}

impl App {
    fn c(&self, c: Color) -> Style {
        if self.no_color { Style::default() } else { Style::default().fg(c) }
    }
    fn bg(&self) -> Option<Color> {
        if self.no_color || !self.cfg.tui.solid_background {
            return None;
        }
        Some(parse_hex(&self.cfg.tui.background).unwrap_or(Color::Rgb(0x12, 0x15, 0x1b)))
    }
    fn block<'a>(&self, title: &str, focus: bool) -> Block<'a> {
        let b = Block::default();
        let b = match self.bg() {
            Some(bg) => b.style(Style::default().bg(bg)),
            None => b,
        };
        b.borders(Borders::ALL)
            .border_type(if focus { BorderType::Thick } else { BorderType::Rounded })
            .border_style(if focus { self.c(ACCENT).add_modifier(Modifier::BOLD) } else { self.c(BORDER) })
            .title(if focus {
                Span::styled(
                    format!(" ▸ {title} "),
                    if self.no_color {
                        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
                    },
                )
            } else {
                Span::styled(format!(" {title} "), self.c(DIM).add_modifier(Modifier::BOLD))
            })
    }
    /// Flight plan: daemon cache first, then this session's on-demand fetch.
    fn plan_for(&self, l: &Launch) -> Option<&FlightPlan> {
        self.cache.flight_plans.get(&l.id).or_else(|| self.plans.get(&l.id).and_then(|p| p.as_ref()))
    }
    fn request_patch(&mut self, url: String) {
        if self.patch_loading.contains(&url) {
            return;
        }
        self.patch_loading.insert(url.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let img = crate::patch::load(&url).ok();
            let _ = tx.send(Msg::Patch(url, img));
        });
    }
    /// On-demand SpaceX plan for the selected launch when the daemon has none (once per session).
    fn maybe_fetch_plan(&mut self) {
        if !self.cfg.spacex_flight_plans {
            return;
        }
        let Some(l) = self.selected().cloned() else { return };
        if !spacex::is_spacex(&l) || self.cache.flight_plans.contains_key(&l.id) || self.plans.contains_key(&l.id) || self.plan_loading.contains(&l.id) {
            return;
        }
        self.plan_loading.insert(l.id.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let p = spacex::fetch_tiles().ok().and_then(|t| spacex::match_tile(&l, &t).cloned()).and_then(|t| spacex::fetch_plan(&t).ok());
            let _ = tx.send(Msg::Plan(l.id, p));
        });
    }
    fn launches(&self) -> &[Launch] {
        &self.cache.launches
    }
    fn selected(&self) -> Option<&Launch> {
        self.table.selected().and_then(|i| self.cache.launches.get(i))
    }
    fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }
    fn starred(&self, l: &Launch) -> bool {
        self.cfg.is_starred(&l.provider, &l.provider_abbrev)
    }

    fn reload_if_changed(&mut self) {
        let p = paths::cache_file();
        let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        if m != self.mtime {
            self.mtime = m;
            self.state = store::load_state(&paths::state_file());
            self.apply_cache(store::load_cache(&p));
        }
    }

    /// Swap in a freshly loaded cache, keeping the in-memory test flight and the selection.
    fn apply_cache(&mut self, cache: Cache) {
        self.remember_selection();
        self.cache = cache;
        self.cache.launches.iter_mut().for_each(sanitize);
        for (k, v) in &self.cache.news {
            self.news.insert(k.clone(), v.clone());
        }
        self.orig_nets = self.cache.launches.iter().map(|l| (l.id.clone(), l.net)).collect();
        self.apply_nudge();
        if self.test.is_some() {
            self.inject_test(Utc::now());
        }
        self.resync_selection();
    }

    /// Wall clock corrected by the daemon's measured skew (0 unless > 1 s).
    fn now(&self) -> DateTime<Utc> {
        Utc::now() + Duration::milliseconds(self.cache.sync.clock_offset_ms)
    }

    /// Re-anchor launches on the user's nudged T-0 (from LL2 NETs as loaded).
    fn apply_nudge(&mut self) {
        for l in self.cache.launches.iter_mut() {
            if let Some(n) = self.orig_nets.get(&l.id) {
                l.net = *n;
            }
            if let Some(t0) = self.nudge.applies(l) {
                l.net = t0;
            }
        }
    }

    /// Launch the nudge keys act on: the live one, else the selection.
    fn nudge_target(&self) -> Option<Launch> {
        live_mission(self, self.now()).map(|x| x.0).or_else(|| self.selected().cloned())
    }

    fn nudge(&mut self, op: char) {
        let Some(mut l) = self.nudge_target() else { return };
        if l.id == testflight::ID {
            self.toast("test flight: nudge not available");
            return;
        }
        if let Some(n) = self.orig_nets.get(&l.id) {
            l.net = *n; // nudge math is relative to LL2's NET
        }
        let now = self.now();
        self.nudge = match op {
            '<' => self.nudge.shifted(&l, -1, now),
            '>' => self.nudge.shifted(&l, 1, now),
            'M' => self.nudge.mark(&l, now),
            _ => crate::nudge::Nudge::default(),
        };
        if !cfg!(test) {
            let _ = crate::nudge::save(&self.nudge);
        }
        self.apply_nudge();
        let off = self.nudge.offset_secs(&l).unwrap_or(0);
        self.toast(match op {
            'M' => format!("liftoff marked now · T-0 {off:+}s vs LL2"),
            '=' => "T-0 nudge cleared (LL2 NET)".into(),
            _ => format!("T-0 {off:+}s vs LL2 · {}", short_mission(&l, 20)),
        });
    }

    fn remember_selection(&mut self) {
        if let Some(id) = self.selected().map(|l| l.id.clone()) {
            self.sel_id = Some(id);
        }
    }

    /// Point the table at `sel_id`; if it vanished use index 0; on first load the next launch.
    fn resync_selection(&mut self) {
        let ls = &self.cache.launches;
        let idx = match &self.sel_id {
            Some(id) => ls.iter().position(|l| &l.id == id).or(if ls.is_empty() { None } else { Some(0) }),
            None => schedule::next_launch(ls, Utc::now()).and_then(|n| ls.iter().position(|l| l.id == n.id)).or(if ls.is_empty() { None } else { Some(0) }),
        };
        self.table.select(idx);
        if let Some(i) = idx {
            self.sel_id = Some(self.cache.launches[i].id.clone());
        }
    }

    fn spawn_fetch(&mut self) {
        if self.fetching {
            return;
        }
        self.fetching = true;
        let cfg = self.cfg.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = daemon::fetch_and_save(&cfg, false).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Fetched(r));
        });
    }

    fn open_news(&mut self) {
        let Some(l) = self.selected().cloned() else { return };
        self.overlay = Overlay::News;
        self.news_sel = 0;
        if self.news.contains_key(&l.id) || self.news_loading.as_deref() == Some(&l.id) {
            return;
        }
        self.news_loading = Some(l.id.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut a = api::fetch_news(&l.id, 8).unwrap_or_default();
            // Fallback: recent free-text search, only for distinctive mission names.
            let generic = l.mission.is_empty() || l.mission.to_lowercase().contains("unknown");
            if a.is_empty() && !generic {
                a = api::search_news(&l.mission, 8).unwrap_or_default();
                for x in &mut a {
                    x.news_site = format!("{} (related)", x.news_site);
                }
            }
            let _ = tx.send(Msg::News(l.id, a));
        });
    }

    fn toggle_test_flight(&mut self) {
        if self.test.take().is_some() {
            self.cache.launches.retain(|l| l.id != testflight::ID);
            self.cache.flight_plans.remove(testflight::ID);
            if !cfg!(test) {
                testflight::stop_marker();
            }
            self.resync_selection();
            self.toast("test flight off");
        } else {
            self.test = Some(TestRun { started: Instant::now(), prev_t: testflight::START, beat: None, started_ms: Utc::now().timestamp_millis() });
            self.update_test(Utc::now());
            self.sel_id = Some(testflight::ID.into());
            self.resync_selection();
            self.details_scroll = 0;
            self.plan_scroll = None;
            self.side = SidePane::FlightPlan;
            self.toast("TEST FLIGHT · looping synthetic Falcon 9 (Ctrl-x t f to stop)");
        }
    }
    /// Insert/refresh the synthetic launch (constant id) at the top; selection follows its id.
    fn inject_test(&mut self, now: DateTime<Utc>) {
        let Some(run) = &self.test else { return };
        let l = testflight::launch(now, run.prev_t);
        match self.cache.launches.iter_mut().find(|x| x.id == testflight::ID) {
            Some(x) => *x = l,
            None => {
                self.cache.launches.insert(0, l);
                self.resync_selection();
            }
        }
        self.cache.flight_plans.entry(testflight::ID.into()).or_insert_with(testflight::plan);
    }
    /// Refresh the synthetic launch in memory (never saved) and fire TEST alerts.
    fn update_test(&mut self, now: DateTime<Utc>) {
        let Some(run) = &mut self.test else { return };
        let t = testflight::sim_t(run.started.elapsed().as_secs_f64());
        let crossed = testflight::crossed(run.prev_t, t);
        run.prev_t = t;
        if !cfg!(test) && run.beat.is_none_or(|b| b.elapsed().as_millis() >= 1000) {
            run.beat = Some(Instant::now());
            if testflight::stopped_since(run.started_ms) {
                // switched off in another TUI: stop here too, without re-writing the marker
                self.test = None;
                self.cache.launches.retain(|l| l.id != testflight::ID);
                self.cache.flight_plans.remove(testflight::ID);
                self.resync_selection();
                self.toast("test flight off (stopped in another window)");
                return;
            }
            testflight::write_marker(now, t);
        }
        self.remember_selection();
        self.inject_test(now);
        if !crossed.is_empty() && !cfg!(test) {
            testflight::deliver(&self.cfg.notify, &crossed);
        }
    }
    fn focused(&self, p: Pane) -> bool {
        self.overlay == Overlay::None && self.focus == p
    }
    fn cycle_focus(&mut self, d: i64) {
        let order: Vec<Pane> = self.visible.iter().map(|v| v.0).collect();
        self.focus = next_focus(&order, self.focus, d);
    }
    /// Scroll the focused pane by `d` lines (Upcoming/Map: move selection).
    fn scroll(&mut self, d: i64) {
        match self.focus {
            Pane::List | Pane::Map => self.move_sel(d),
            Pane::Details => self.details_scroll = clamp_scroll(self.details_scroll, d, self.details_max),
            Pane::Telemetry => {}
            Pane::Plan => self.plan_scroll = Some(clamp_scroll(self.plan_scroll.unwrap_or(self.plan_auto), d, self.plan_max)),
        }
    }
    fn scroll_home(&mut self, end: bool) {
        match self.focus {
            Pane::List | Pane::Map => {
                if end {
                    self.move_sel(i64::MAX / 2)
                } else {
                    self.move_sel(i64::MIN / 2)
                }
            }
            Pane::Details => self.details_scroll = if end { self.details_max } else { 0 },
            Pane::Telemetry => {}
            Pane::Plan => self.plan_scroll = Some(if end { self.plan_max } else { 0 }),
        }
    }
    fn move_sel(&mut self, d: i64) {
        let n = self.launches().len() as i64;
        if n == 0 {
            return;
        }
        let i = self.table.selected().unwrap_or(0) as i64;
        self.table.select(Some((i + d).clamp(0, n - 1) as usize));
        self.details_scroll = 0;
        self.plan_scroll = None;
    }

    fn on_key(&mut self, k: KeyEvent) {
        self.on_key_inner(k);
        self.remember_selection();
    }
    fn on_key_inner(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.overlay == Overlay::News {
            let n = self.selected().and_then(|l| self.news.get(&l.id)).map(|v| v.len()).unwrap_or(0);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => self.overlay = Overlay::None,
                KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => self.news_sel = (self.news_sel + 1).min(n.saturating_sub(1)),
                KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => self.news_sel = self.news_sel.saturating_sub(1),
                KeyCode::PageDown => self.news_sel = (self.news_sel + 5).min(n.saturating_sub(1)),
                KeyCode::PageUp => self.news_sel = self.news_sel.saturating_sub(5),
                KeyCode::Char('g') | KeyCode::Home => self.news_sel = 0,
                KeyCode::Char('G') | KeyCode::End => self.news_sel = n.saturating_sub(1),
                KeyCode::Enter | KeyCode::Char('o') => {
                    if let Some(a) = self.selected().and_then(|l| self.news.get(&l.id)).and_then(|v| v.get(self.news_sel)) {
                        open_url(&a.url);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.overlay == Overlay::Help {
            self.overlay = Overlay::None;
            return;
        }
        // hidden chord: Ctrl-x, t, f (within 3 s)
        if ctrl && k.code == KeyCode::Char('x') {
            self.chord = Some((Instant::now(), String::new()));
            return;
        }
        if let Some((at, seq)) = self.chord.take() {
            if at.elapsed().as_secs() < 3 {
                match (seq.as_str(), k.code) {
                    ("", KeyCode::Char('t')) => {
                        self.chord = Some((at, "t".into()));
                        return;
                    }
                    ("t", KeyCode::Char('f')) => {
                        self.toggle_test_flight();
                        return;
                    }
                    _ => {}
                }
            }
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => self.cycle_focus(1),
            KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => self.cycle_focus(-1),
            KeyCode::Char('j') | KeyCode::Down => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::Char('d') if ctrl => self.scroll(10),
            KeyCode::Char('u') if ctrl => self.scroll(-10),
            KeyCode::PageDown => self.scroll(10),
            KeyCode::PageUp => self.scroll(-10),
            KeyCode::Char('g') | KeyCode::Home => self.scroll_home(false),
            KeyCode::Char('G') | KeyCode::End => self.scroll_home(true),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('n') => self.open_news(),
            KeyCode::Char('t') => self.side = self.side.toggle(SidePane::Telemetry),
            KeyCode::Char('f') => self.side = self.side.toggle(SidePane::FlightPlan),
            KeyCode::Char('<') => self.nudge('<'),
            KeyCode::Char('>') => self.nudge('>'),
            KeyCode::Char('M') => self.nudge('M'),
            KeyCode::Char('=') => self.nudge('='),
            KeyCode::Char(']') | KeyCode::Char('J') => self.details_scroll = clamp_scroll(self.details_scroll, 3, self.details_max),
            KeyCode::Char('[') | KeyCode::Char('K') => self.details_scroll = clamp_scroll(self.details_scroll, -3, self.details_max),
            KeyCode::Char('p') => match self.selected().and_then(|l| self.plan_for(l)).map(|p| p.url.clone()) {
                Some(u) => {
                    open_url(&u);
                    self.toast(format!("opening {u}"));
                }
                None => self.toast("no spacex.com mission page for this launch"),
            },
            KeyCode::Char('w') => match self.selected().and_then(|l| l.webcast_url.clone().or(l.info_url.clone())) {
                Some(u) => {
                    open_url(&u);
                    self.toast(format!("opening {u}"));
                }
                None => self.toast("no webcast listed yet"),
            },
            KeyCode::Char('s') => {
                if let Some(l) = self.selected().cloned() {
                    let name = if l.provider.is_empty() { l.provider_abbrev.clone() } else { l.provider.clone() };
                    // remove abbrev form too so toggling off works for either spelling
                    let on = if self.starred(&l) {
                        self.cfg.starred_providers.retain(|p| !p.eq_ignore_ascii_case(&l.provider) && !p.eq_ignore_ascii_case(&l.provider_abbrev));
                        false
                    } else {
                        self.cfg.toggle_star(&name)
                    };
                    let saved = self.cfg.save();
                    self.toast(match saved {
                        Ok(_) => format!("{} {name} — T-24h/T-1h alerts {}", if on { "★" } else { "☆" }, if on { "on" } else { "off" }),
                        Err(e) => format!("could not save config: {e}"),
                    });
                }
            }
            KeyCode::Char('r') => {
                let now = Utc::now();
                let slots = daemon::reserved_slots(&self.cfg, &self.cache, &self.state, now);
                if !slots.is_empty() && !planner::allowed_now(&self.state.budget.calls, &slots, self.cfg.max_calls_per_hour, now) {
                    let next = planner::next_unreserved(&self.state.budget.calls, &slots, self.cfg.max_calls_per_hour, now, now);
                    self.toast(format!(
                        "refresh held: calls reserved for final syncs (free at {}) — showing cache",
                        next.with_timezone(&Local).format("%H:%M:%S")
                    ));
                    return;
                }
                let _ = store::write_atomic(&daemon::refresh_request_file(), b"1");
                let stale = self.cache.fetched_at.is_none_or(|t| Utc::now() - t > Duration::minutes(25));
                if stale {
                    self.toast("daemon looks idle — fetching directly");
                    self.spawn_fetch();
                } else {
                    self.toast("refresh requested (daemon, budget-aware)");
                }
            }
            _ => {}
        }
    }

    fn on_mouse(&mut self, m: crossterm::event::MouseEvent) {
        self.on_mouse_inner(m);
        self.remember_selection();
    }
    fn on_mouse_inner(&mut self, m: crossterm::event::MouseEvent) {
        let inside = |r: Rect| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if self.overlay == Overlay::News => {
                let n = self.selected().and_then(|l| self.news.get(&l.id)).map(|v| v.len()).unwrap_or(0);
                self.news_sel =
                    if m.kind == MouseEventKind::ScrollDown { (self.news_sel + 1).min(n.saturating_sub(1)) } else { self.news_sel.saturating_sub(1) };
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if self.overlay == Overlay::None => {
                // wheel scrolls the pane under the pointer (and focuses it), else the focused pane
                if let Some((p, _)) = self.visible.iter().find(|(_, r)| inside(*r)) {
                    self.focus = *p;
                }
                self.scroll(if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 });
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.overlay != Overlay::None {
                    self.overlay = Overlay::None;
                    return;
                }
                if let Some((p, _)) = self.visible.iter().find(|(_, r)| inside(*r)) {
                    self.focus = *p;
                }
                if inside(self.list_area) {
                    let row = m.row as i64 - self.list_area.y as i64 - 2; // border + header
                    if row >= 0 {
                        let i = self.table.offset() + row as usize;
                        if i < self.launches().len() {
                            self.table.select(Some(i));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn open_url(u: &str) {
    let _ = std::process::Command::new("xdg-open").arg(u).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

// ------------------------------------------------------------------ big font

const GLYPHS: &[(char, [&str; 5])] = &[
    ('0', ["███", "█ █", "█ █", "█ █", "███"]),
    ('1', [" ██", "  █", "  █", "  █", "  █"]),
    ('2', ["███", "  █", "███", "█  ", "███"]),
    ('3', ["███", "  █", " ██", "  █", "███"]),
    ('4', ["█ █", "█ █", "███", "  █", "  █"]),
    ('5', ["███", "█  ", "███", "  █", "███"]),
    ('6', ["███", "█  ", "███", "█ █", "███"]),
    ('7', ["███", "  █", "  █", "  █", "  █"]),
    ('8', ["███", "█ █", "███", "█ █", "███"]),
    ('9', ["███", "█ █", "███", "  █", "███"]),
    ('T', ["███", " █ ", " █ ", " █ ", " █ "]),
    ('-', ["   ", "   ", "███", "   ", "   "]),
    ('+', ["   ", " █ ", "███", " █ ", "   "]),
    (':', [" ", "▪", " ", "▪", " "]),
    ('d', ["  █", "  █", "███", "█ █", "███"]),
    ('h', ["█  ", "█  ", "███", "█ █", "█ █"]),
    (' ', [" ", " ", " ", " ", " "]),
];

pub fn big_text(s: &str, blink_colon: bool) -> Vec<String> {
    let mut rows = vec![String::new(); 5];
    for ch in s.chars() {
        let g = GLYPHS.iter().find(|(c, _)| *c == ch).map(|(_, g)| *g).unwrap_or([" "; 5]);
        for (r, row) in rows.iter_mut().enumerate() {
            let piece = if ch == ':' && blink_colon { " " } else { g[r] };
            row.push_str(piece);
            row.push(' ');
        }
    }
    rows
}

fn twinkle(x: u16, y: u16, tick: u64) -> Option<char> {
    let h = (x as u64).wrapping_mul(73856093) ^ (y as u64).wrapping_mul(19349663);
    if h % 23 != 0 {
        return None;
    }
    let phase = (h / 23 + tick / 4) % 12;
    Some(match phase {
        0 => '✦',
        1 | 11 => '·',
        2..=4 => '.',
        _ => ' ',
    })
}

// ------------------------------------------------------------------ draw

fn draw(f: &mut Frame, app: &mut App) {
    let now = app.now();
    let area = f.area();
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(10), Constraint::Length(7), Constraint::Length(1)]).split(area);
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(44), Constraint::Percentage(56)]).split(rows[0]);
    let left = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(13), Constraint::Min(5)]).split(cols[0]);
    let right = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(45), Constraint::Percentage(55)]).split(cols[1]);

    // Paint every cell first: nothing from a previous frame or a stray write survives.
    f.render_widget(Clear, area);
    if let Some(bg) = app.bg() {
        f.render_widget(Block::default().style(Style::default().bg(bg)), area);
    }
    app.maybe_fetch_plan();
    app.visible.clear();
    draw_hero(f, app, left[0], now);
    draw_list(f, app, left[1], now);
    app.visible.push((Pane::List, left[1]));
    let details_slot = app.visible.len();
    draw_map(f, app, right[0], now);
    let side: Option<(Pane, fn(&mut Frame, &mut App, Rect, DateTime<Utc>))> = match app.side {
        SidePane::Telemetry => Some((Pane::Telemetry, |f, a, r, _| draw_telemetry(f, a, r))),
        SidePane::FlightPlan => Some((Pane::Plan, draw_plan)),
        SidePane::Hidden => None,
    };
    match side {
        Some((pane, g)) => {
            let parts = if right[1].width >= 70 {
                Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(54), Constraint::Percentage(46)]).split(right[1])
            } else {
                Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(50), Constraint::Percentage(50)]).split(right[1])
            };
            draw_details(f, app, parts[0], now);
            g(f, app, parts[1], now);
            app.visible.insert(details_slot, (Pane::Details, parts[0]));
            app.visible.insert(details_slot + 1, (pane, parts[1]));
        }
        None => {
            draw_details(f, app, right[1], now);
            app.visible.insert(details_slot, (Pane::Details, right[1]));
        }
    }
    app.visible.push((Pane::Map, right[0]));
    if !app.visible.iter().any(|v| v.0 == app.focus) {
        app.focus = Pane::List;
    }
    draw_timeline(f, app, rows[1], now);
    draw_footer(f, app, rows[2], now);
    match app.overlay {
        Overlay::Help => draw_help(f, app, area),
        Overlay::News => draw_news(f, app, area),
        Overlay::None => {}
    }
}

fn draw_hero(f: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let block = app.block("Next Launch", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let test_l = app.test.as_ref().and_then(|_| app.launches().iter().find(|l| l.id == testflight::ID));
    let Some(l) = test_l.or_else(|| schedule::next_launch(app.launches(), now)) else {
        let msg = if app.fetching { "fetching launches…" } else { "no launches cached — press r" };
        f.render_widget(Paragraph::new(msg).style(app.c(DIM)).alignment(Alignment::Center), inner);
        return;
    };
    // starfield
    if !app.reduced_motion && !app.no_color {
        let buf = f.buffer_mut();
        for y in inner.y..inner.y + inner.height {
            for x in inner.x..inner.x + inner.width {
                if let Some(ch) = twinkle(x, y, app.tick) {
                    buf[(x, y)].set_char(ch).set_fg(DIM);
                }
            }
        }
    }
    let col = status_color(&l.status_class);
    let pulse = l.status_class == "flight" && app.tick % 8 < 4;
    let cd = schedule::display_countdown(l, now);
    let mut lines: Vec<Line> = vec![];
    let star = if app.starred(l) { "★ " } else { "" };
    lines.push(Line::from(vec![
        Span::styled(format!("{star}{}", l.short_rocket()), app.c(FG).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {}", if l.mission.is_empty() { &l.name } else { &l.mission }), app.c(FG)),
    ]));
    lines.push(Line::from(Span::styled(format!("{} · {}", l.provider, l.pad), app.c(DIM))));
    lines.push(Line::raw(""));
    let blink = !app.reduced_motion && (now - l.net).num_hours().abs() < 1 && app.tick % 4 >= 2;
    let big = big_text(&cd, blink);
    let big_w = big[0].chars().count() as u16;
    let mut style = app.c(col).add_modifier(Modifier::BOLD);
    if pulse {
        style = style.add_modifier(Modifier::DIM);
    }
    if big_w <= inner.width && cd.starts_with('T') {
        for r in big {
            lines.push(Line::from(Span::styled(r, style)));
        }
    } else {
        lines.push(Line::from(Span::styled(cd, style)));
    }
    lines.push(Line::raw(""));
    let mut meta = vec![Span::styled(
        format!(" {} ", l.status_name.to_uppercase()),
        if app.no_color { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default().bg(col).fg(Color::Black).add_modifier(Modifier::BOLD) },
    )];
    if let Some(p) = l.probability {
        meta.push(Span::styled(format!("  ☁ {p}% go"), app.c(FG)));
    }
    if l.webcast_live {
        let on = app.tick % 6 < 3 || app.reduced_motion;
        meta.push(Span::styled("  ● LIVE", app.c(if on { Color::Red } else { DIM }).add_modifier(Modifier::BOLD)));
    }
    lines.push(Line::from(meta));
    if let Some(w) = &l.weather_concerns {
        lines.push(Line::from(Span::styled(format!("weather: {w}"), app.c(DIM))));
    } else {
        lines.push(Line::from(Span::styled(format!("NET {}", l.net.with_timezone(&Local).format("%a %b %-d %H:%M")), app.c(DIM))));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

fn short_countdown(l: &Launch, now: DateTime<Utc>) -> String {
    let secs = -schedule::rel_secs(l.net, now);
    if !l.precise() && secs.abs() > 3600 {
        return l.net.with_timezone(&Local).format("%b %-d").to_string();
    }
    let a = secs.abs();
    let s = if a >= 86400 { format!("{}d{:02}h", a / 86400, (a % 86400) / 3600) } else { format!("{:02}:{:02}:{:02}", a / 3600, (a % 3600) / 60, a % 60) };
    format!("{}{s}", if secs >= 0 { "-" } else { "+" })
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    app.list_area = area;
    let rows: Vec<Row> = app
        .launches()
        .iter()
        .map(|l| {
            let col = status_color(&l.status_class);
            Row::new(vec![
                Cell::from(if app.starred(l) { "★" } else { " " }).style(app.c(ACCENT)),
                Cell::from(short_countdown(l, now)).style(app.c(col)),
                Cell::from(l.short_rocket()).style(app.c(FG)),
                Cell::from(if l.mission.is_empty() { l.name.clone() } else { l.mission.clone() }),
                Cell::from(if l.provider_abbrev.is_empty() { l.provider.clone() } else { l.provider_abbrev.clone() }).style(app.c(DIM)),
                Cell::from(l.status_abbrev.clone()).style(app.c(col)),
            ])
        })
        .collect();
    let n = rows.len();
    let title = format!("Upcoming · {n} worldwide");
    let t = Table::new(
        rows,
        [Constraint::Length(1), Constraint::Length(9), Constraint::Length(14), Constraint::Min(10), Constraint::Length(6), Constraint::Length(7)],
    )
    .header(Row::new(vec!["", "T", "Vehicle", "Mission", "LSP", "Status"]).style(app.c(DIM).add_modifier(Modifier::BOLD)))
    .row_highlight_style(if app.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().bg(Color::Rgb(0x2a, 0x30, 0x3c)).add_modifier(Modifier::BOLD)
    })
    .highlight_symbol("▌")
    .block(app.block(&title, app.focused(Pane::List)));
    f.render_stateful_widget(t, area, &mut app.table);
    let vis = area.height.saturating_sub(3) as usize;
    let sel = app.table.selected().unwrap_or(0);
    scrollbar(f, area, n, app.table.offset().max(sel.saturating_sub(vis.saturating_sub(1))).min(n.saturating_sub(vis)), vis);
}

// ------------------------------------------------------------------ map

/// Project lon/lat onto a cell inside `r` (same transform the Canvas uses).
fn project(r: Rect, lon: f64, lat: f64) -> (u16, u16) {
    let fx = ((lon + 180.0) / 360.0).clamp(0.0, 1.0);
    let fy = ((90.0 - lat) / 180.0).clamp(0.0, 1.0);
    (r.x + (fx * (r.width.saturating_sub(1)) as f64).round() as u16, r.y + (fy * (r.height.saturating_sub(1)) as f64).round() as u16)
}

/// A pad label: short pad name (e.g. "SLC-40", "LC-39A") plus launch count.
pub fn pad_label(pad: &str, n: usize) -> String {
    let p = pad.split(',').next().unwrap_or(pad).trim();
    let p = p.strip_prefix("Space Launch Complex ").map(|x| format!("SLC-{x}")).unwrap_or_else(|| p.to_string());
    let p = p.strip_prefix("Launch Complex ").map(|x| format!("LC-{x}")).unwrap_or(p);
    let p: String = p.chars().take(16).collect();
    if n > 1 { format!("{p} ×{n}") } else { p }
}

/// Greedy non-overlapping label placement. Each item: (x, y, width). Tries to the
/// right, left, above and below the marker. Returns chosen (x, y) or None.
pub fn place_labels(items: &[(u16, u16, u16)], area: Rect, taken: &mut Vec<Rect>) -> Vec<Option<(u16, u16)>> {
    items
        .iter()
        .map(|&(x, y, w)| {
            let cands = [
                (x as i32 + 2, y as i32),
                (x as i32 - w as i32 - 1, y as i32),
                (x as i32 - w as i32 / 2, y as i32 - 1),
                (x as i32 - w as i32 / 2, y as i32 + 1),
                (x as i32 + 2, y as i32 - 1),
                (x as i32 + 2, y as i32 + 1),
            ];
            for (cx, cy) in cands {
                if cx < area.x as i32 || cy < area.y as i32 || cx + w as i32 > (area.x + area.width) as i32 || cy >= (area.y + area.height) as i32 {
                    continue;
                }
                let r = Rect { x: cx as u16, y: cy as u16, width: w, height: 1 };
                let grown = Rect { x: r.x.saturating_sub(1), width: r.width + 2, ..r };
                if taken.iter().all(|t| !t.intersects(grown)) {
                    taken.push(r);
                    return Some((r.x, r.y));
                }
            }
            None
        })
        .collect()
}

fn draw_map(f: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let sel = app.selected().cloned();
    let pulse = if app.reduced_motion { true } else { app.tick % 6 < 4 };
    let title = sel.as_ref().map(|l| format!("Pads · {}", l.location)).unwrap_or_else(|| "Pads".into());
    let no_color = app.no_color;
    let land = if no_color { Color::Reset } else { LAND };
    let block = app.block(&title, app.focused(Pane::Map));
    let inner = block.inner(area);
    let mut canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds([-180.0, 180.0])
        .y_bounds([-90.0, 90.0])
        .paint(move |ctx| ctx.draw(&Map { resolution: MapResolution::High, color: land }));
    if let Some(bg) = app.bg() {
        canvas = canvas.background_color(bg);
    }
    f.render_widget(canvas, area);
    if inner.width < 8 || inner.height < 4 {
        return;
    }
    let buf = f.buffer_mut();
    // bold land glyphs so the coastline survives transparency/blur
    if !no_color {
        for y in inner.y..inner.y + inner.height {
            for x in inner.x..inner.x + inner.width {
                let c = &mut buf[(x, y)];
                if c.symbol() != " " {
                    c.modifier.insert(Modifier::BOLD);
                }
            }
        }
    }
    // group upcoming launches by pad
    let mut pads: Vec<(f64, f64, String, usize, String, bool)> = vec![]; // lon, lat, pad, count, status, selected
    for l in app.launches().iter().filter(|l| l.net > now - Duration::hours(2)) {
        let (Some(lon), Some(lat)) = (l.lon, l.lat) else { continue };
        let is_sel = sel.as_ref().is_some_and(|s| s.id == l.id);
        match pads.iter_mut().find(|p| (p.0 - lon).abs() < 0.05 && (p.1 - lat).abs() < 0.05) {
            Some(p) => {
                p.3 += 1;
                p.5 |= is_sel;
            }
            None => pads.push((lon, lat, l.pad.clone(), 1, l.status_class.clone(), is_sel)),
        }
    }
    // selected pad first so its label always wins
    pads.sort_by_key(|p| (!p.5, std::cmp::Reverse(p.3)));
    let marker_style = |p: &(f64, f64, String, usize, String, bool)| {
        if no_color {
            Style::default().add_modifier(Modifier::BOLD)
        } else if p.5 {
            Style::default().fg(status_color(&p.4)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        }
    };
    let mut taken: Vec<Rect> = pads
        .iter()
        .map(|p| {
            let (x, y) = project(inner, p.0, p.1);
            Rect { x, y, width: 1, height: 1 }
        })
        .collect();
    // landing zone + downrange hint for the selected launch
    if let Some(l) = &sel {
        for b in &l.boosters {
            if let (Some(lon), Some(lat), Some(plon), Some(plat)) = (b.landing_lon, b.landing_lat, l.lon, l.lat) {
                let (x0, y0) = project(inner, plon, plat);
                let (x1, y1) = project(inner, lon, lat);
                let steps = (x0.abs_diff(x1)).max(y0.abs_diff(y1)).max(1);
                for i in 1..steps {
                    let t = i as f64 / steps as f64;
                    let x = (x0 as f64 + (x1 as f64 - x0 as f64) * t).round() as u16;
                    let y = (y0 as f64 + (y1 as f64 - y0 as f64) * t).round() as u16;
                    buf[(x, y)].set_char('·').set_fg(if no_color { Color::Reset } else { FG });
                }
                buf[(x1, y1)]
                    .set_char('⊕')
                    .set_style(Style::default().fg(if no_color { Color::Reset } else { Color::Rgb(0x7a, 0xc7, 0xd6) }).add_modifier(Modifier::BOLD));
                taken.push(Rect { x: x1, y: y1, width: 1, height: 1 });
            }
        }
    }
    let items: Vec<(u16, u16, u16)> = pads
        .iter()
        .map(|p| {
            let (x, y) = project(inner, p.0, p.1);
            (x, y, pad_label(&p.2, p.3).chars().count() as u16)
        })
        .collect();
    let spots = place_labels(&items, inner, &mut taken);
    let label_bg = app.bg().unwrap_or(Color::Reset);
    for (p, spot) in pads.iter().zip(spots) {
        let (x, y) = project(inner, p.0, p.1);
        let sym = if p.5 { if pulse { "◉" } else { "○" } } else { "●" };
        buf[(x, y)].set_symbol(sym).set_style(marker_style(p));
        if let Some((lx, ly)) = spot {
            let st = if no_color {
                if p.5 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() }
            } else if p.5 {
                Style::default().fg(Color::Black).bg(status_color(&p.4)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG).bg(label_bg)
            };
            buf.set_string(lx, ly, pad_label(&p.2, p.3), st);
        }
    }
}

// ------------------------------------------------------------------ details

fn gauge_spans(app: &App, frac: f64, width: usize, col: Color) -> Vec<Span<'static>> {
    let fill = ((frac.clamp(0.0, 1.0)) * width as f64).round() as usize;
    vec![Span::styled("█".repeat(fill), app.c(col)), Span::styled("░".repeat(width - fill), app.c(DIM))]
}

fn fmt_num(v: Option<f64>, unit: &str) -> Option<String> {
    v.map(|x| if x >= 1000.0 { format!("{:.1}k {unit}", x / 1000.0) } else { format!("{x} {unit}") })
}

/// Window bar: "|----▲------|" with NET position and "now" position when inside.
pub fn window_bar(start: DateTime<Utc>, end: DateTime<Utc>, net: DateTime<Utc>, now: DateTime<Utc>, width: usize) -> String {
    let span = (end - start).num_seconds().max(1) as f64;
    let pos = |t: DateTime<Utc>| (((t - start).num_seconds() as f64 / span) * (width.saturating_sub(1)) as f64).round();
    let mut v = vec!['─'; width.max(2)];
    v[0] = '├';
    let last = v.len() - 1;
    v[last] = '┤';
    let n = pos(net);
    if n >= 0.0 && (n as usize) < v.len() {
        v[n as usize] = '▲';
    }
    let x = pos(now);
    if x >= 0.0 && (x as usize) < v.len() {
        v[x as usize] = '│';
    }
    v.into_iter().collect()
}

fn milestone_source<'a>(app: &'a App, l: &'a Launch) -> (&'a [Milestone], &'static str) {
    match app.plan_for(l) {
        Some(p) if !p.events.is_empty() => (&p.events, "spacex.com"),
        _ => (&l.milestones, "LL2"),
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    let block = app.block("Details", app.focused(Pane::Details));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(l) = app.selected().cloned() else { return };
    // mission patch on the right when there is room
    let mut text_area = inner;
    if app.cfg.tui.mission_patch && inner.width >= 56 && inner.height >= 8 {
        if let Some(url) = &l.patch_url {
            let pw = 18u16.min(inner.width / 3);
            let ph = 9u16.min(inner.height);
            match app.patches.get(url) {
                Some(Some(img)) => {
                    let lines = crate::patch::render(img, pw, ph);
                    let r = Rect { x: inner.x + inner.width - pw, y: inner.y, width: pw, height: ph };
                    f.render_widget(Paragraph::new(lines), r);
                    text_area.width -= pw + 1;
                }
                Some(None) => {}
                None => app.request_patch(url.clone()),
            }
        }
    }
    let col = status_color(&l.status_class);
    let w = text_area.width.saturating_sub(10).clamp(10, 40) as usize;
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<10}"), app.c(DIM)), Span::styled(v, app.c(FG))]);
    let head = |t: &str| Line::from(Span::styled(format!("─ {t} "), app.c(ACCENT).add_modifier(Modifier::BOLD)));
    let mut lines = vec![
        Line::from(Span::styled(l.name.clone(), app.c(FG).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("● ", app.c(col)),
            Span::styled(format!("{}  ", l.status_name), app.c(FG).add_modifier(Modifier::BOLD)),
            Span::styled(schedule::display_countdown(&l, now), app.c(FG)),
        ]),
        kv("NET", format!("{} ({})", l.net.with_timezone(&Local).format("%a %b %-d %H:%M %Z"), l.net_precision)),
    ];
    if let (Some(a), Some(b)) = (l.window_start, l.window_end) {
        if a != b {
            lines.push(kv(
                "Window",
                format!("{}–{}  ({}m)", a.with_timezone(&Local).format("%H:%M"), b.with_timezone(&Local).format("%H:%M"), (b - a).num_minutes()),
            ));
            lines.push(Line::from(vec![Span::raw(" ".repeat(10)), Span::styled(window_bar(a, b, l.net, now, w), app.c(FG))]));
        } else {
            lines.push(kv("Window", "instantaneous".into()));
        }
    }
    if let Some(p) = l.probability {
        let gc = if p >= 70 {
            status_color("go")
        } else if p >= 40 {
            status_color("hold")
        } else {
            status_color("failure")
        };
        let mut s = vec![Span::styled(format!("{:<10}", "Weather"), app.c(DIM))];
        s.extend(gauge_spans(app, p as f64 / 100.0, w.min(20), gc));
        s.push(Span::styled(format!(" {p}% go"), app.c(FG)));
        lines.push(Line::from(s));
    }
    if let Some(wc) = &l.weather_concerns {
        lines.push(kv("Concerns", clean(wc)));
    }
    lines.push(kv(
        "Webcast",
        match &l.webcast_url {
            Some(u) => format!("{}{u}", if l.webcast_live { "● LIVE " } else { "" }),
            None => "—".into(),
        },
    ));
    if let Some(n) = app.news.get(&l.id) {
        lines.push(kv("News", format!("{} article(s) — press n", n.len())));
    }
    // countdown to milestones
    let (ms, src) = milestone_source(app, &l);
    let t_rel = schedule::rel_secs(l.net, now);
    if !ms.is_empty() {
        lines.push(head(&format!("Milestones ({src})")));
        let start = crate::model::current_milestone(ms, t_rel).unwrap_or(0);
        for m in ms.iter().skip(start).take(4) {
            let until = m.offset_secs - t_rel;
            let when = if until > 0 { format!("in {}", short_dur(until)) } else { "now".into() };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<10}", crate::model::fmt_rel(m.offset_secs)), app.c(DIM)),
                Span::styled(format!("{:<14}", m.label), app.c(FG)),
                Span::styled(when, app.c(DIM)),
            ]));
        }
    }
    lines.push(head("Mission"));
    if !l.mission_type.is_empty() || !l.orbit.is_empty() {
        lines.push(kv("Type", [l.mission_type.as_str(), l.orbit.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ")));
    }
    let desc = clean(&l.mission_description);
    lines.push(Line::from(Span::styled(if desc.is_empty() { "No mission description yet.".into() } else { desc }, app.c(FG))));
    lines.push(head("Vehicle"));
    let r = &l.rocket_stats;
    lines.push(kv("Rocket", if l.rocket_full_name.is_empty() { l.rocket.clone() } else { l.rocket_full_name.clone() }));
    let dims: Vec<String> = [
        r.length_m.map(|x| format!("{x} m tall")),
        r.diameter_m.map(|x| format!("{x} m ⌀")),
        r.launch_mass_t.map(|x| format!("{x} t")),
        r.thrust_kn.map(|x| format!("{x} kN")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !dims.is_empty() {
        lines.push(kv("Size", dims.join(" · ")));
    }
    let cap: Vec<String> = [fmt_num(r.leo_kg, "kg LEO"), fmt_num(r.gto_kg, "kg GTO")].into_iter().flatten().collect();
    if !cap.is_empty() {
        lines.push(kv("Payload", cap.join(" · ")));
    }
    if let (Some(t), Some(s)) = (r.total.filter(|t| *t > 0), r.successes) {
        let mut v = format!("{s}/{t} successful");
        if let Some(c) = r.consecutive {
            v.push_str(&format!(" · {c} in a row"));
        }
        lines.push(kv("Record", v));
    }
    if let (Some(a), Some(ok)) = (r.landings_attempted, r.landings_ok) {
        if a > 0 {
            lines.push(kv("Landings", format!("{ok}/{a}")));
        }
    }
    for b in &l.boosters {
        lines.push(head(&format!("Booster {}", if b.serial.is_empty() { b.kind.clone() } else { b.serial.clone() })));
        let mut v = vec![];
        if let Some(n) = b.flight_number {
            v.push(format!("flight #{n}"));
        }
        if b.reused {
            v.push("flight-proven".into());
        }
        if !b.turnaround.is_empty() {
            v.push(format!("turnaround {}", b.turnaround));
        }
        if !v.is_empty() {
            lines.push(kv("Flight", v.join(" · ")));
        }
        if b.landing_attempt {
            let loc = if b.landing_abbrev.is_empty() { b.landing_location.clone() } else { format!("{} ({})", b.landing_location, b.landing_abbrev) };
            let mut s = format!("{} {}", b.landing_type, loc);
            if let Some(d) = b.downrange_km {
                s.push_str(&format!(" · {d:.0} km downrange"));
            }
            lines.push(kv("Landing", s));
            match b.landing_success {
                Some(true) => lines.push(kv("Result", "✓ landed".into())),
                Some(false) => lines.push(kv("Result", "✗ lost".into())),
                None => {}
            }
        } else if !b.landing_description.is_empty() || !b.landing_type.is_empty() {
            lines.push(kv("Landing", "expendable / no attempt".into()));
        }
    }
    if let Some(p) = app.plan_for(&l) {
        if !p.recovery.is_empty() {
            lines.push(kv("Recovery", p.recovery.clone()));
        }
    }
    lines.push(head(&format!("Provider · {}", l.provider)));
    let ps = &l.provider_stats;
    if let (Some(t), Some(s)) = (ps.total, ps.successes) {
        let mut v = format!("{s}/{t} successful");
        if let Some(fl) = ps.failures {
            v.push_str(&format!(" · {fl} failed"));
        }
        lines.push(kv("Launches", v));
        if t > 0 {
            let mut sp = vec![Span::styled(format!("{:<10}", ""), app.c(DIM))];
            sp.extend(gauge_spans(app, s as f64 / t as f64, w.min(20), status_color("go")));
            sp.push(Span::styled(format!(" {:.1}%", 100.0 * s as f64 / t as f64), app.c(FG)));
            lines.push(Line::from(sp));
        }
    }
    let extra: Vec<String> =
        [ps.consecutive.map(|c| format!("{c} in a row")), ps.pending.map(|c| format!("{c} pending")), ps.founded.map(|c| format!("since {c}"))]
            .into_iter()
            .flatten()
            .collect();
    if !extra.is_empty() {
        lines.push(kv("", extra.join(" · ")));
    }
    lines.push(head("Pad"));
    lines.push(kv("Pad", format!("{} · {}", l.pad, l.location)));
    if let Some(n) = l.pad_launch_count {
        lines.push(kv("History", format!("{n} launches from this pad")));
    }
    if let Some(n) = l.orbital_attempt_of_year {
        lines.push(kv("Year", format!("orbital attempt #{n} worldwide this year")));
    }
    let para = Paragraph::new(lines).wrap(Wrap { trim: true });
    let total = para.line_count(text_area.width) as u16;
    app.details_max = total.saturating_sub(text_area.height);
    app.details_scroll = app.details_scroll.min(app.details_max);
    f.render_widget(para.scroll((app.details_scroll, 0)), text_area);
    scrollbar(f, area, total as usize, app.details_scroll as usize, text_area.height as usize);
}

/// Vertical scrollbar on the pane's right border, only when content overflows.
fn scrollbar(f: &mut Frame, area: Rect, total: usize, pos: usize, visible: usize) {
    if total <= visible || area.height < 3 {
        return;
    }
    let mut st = ScrollbarState::new(total.saturating_sub(visible)).position(pos).viewport_content_length(visible);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("┃")
            .thumb_style(Style::default().fg(ACCENT)),
        area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
        &mut st,
    );
}

fn short_dur(s: i64) -> String {
    let s = s.abs();
    if s >= 86400 {
        format!("{}d{}h", s / 86400, (s % 86400) / 3600)
    } else if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

// ------------------------------------------------------------------ flight plan

fn draw_plan(f: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    let Some(l) = app.selected().cloned() else {
        f.render_widget(app.block("Flight Plan", app.focused(Pane::Plan)), area);
        return;
    };
    let plan = app.plan_for(&l).cloned();
    let title = match &plan {
        Some(p) => format!("Flight Plan · spacex.com/launches/{}", p.slug),
        None => "Flight Plan".into(),
    };
    let block = app.block(&title, app.focused(Pane::Plan));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (events, src): (Vec<Milestone>, &str) = match &plan {
        Some(p) if !p.events.is_empty() => (p.events.clone(), "spacex.com"),
        _ => (l.milestones.clone(), "LL2 timeline"),
    };
    let mut lines: Vec<Line> = vec![];
    if let Some(p) = &plan {
        let mut v = vec![];
        if !p.vehicle.is_empty() {
            v.push(p.vehicle.clone());
        }
        if !p.launch_site.is_empty() {
            v.push(p.launch_site.clone());
        }
        if !p.return_site.is_empty() {
            v.push(format!("return: {}", p.return_site));
        }
        lines.push(Line::from(Span::styled(v.join(" · "), app.c(FG))));
    } else if spacex::is_spacex(&l) {
        let msg = if app.plan_loading.contains(&l.id) { "loading spacex.com flight plan…" } else { "no spacex.com flight plan yet — showing LL2 timeline" };
        lines.push(Line::from(Span::styled(msg, app.c(DIM))));
    } else {
        lines.push(Line::from(Span::styled("provider timeline from LL2 (SpaceX launches get spacex.com plans)", app.c(DIM))));
    }
    if events.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled("no published timeline for this launch", app.c(DIM))));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    }
    let t_rel = schedule::rel_secs(l.net, now);
    let live =
        l.precise() && t_rel > events.first().map(|e| e.offset_secs).unwrap_or(0) - 3600 && t_rel < events.last().map(|e| e.offset_secs).unwrap_or(0) + 1800;
    let cur = if live { crate::model::current_milestone(&events, t_rel) } else { None };
    lines.push(Line::from(vec![
        Span::styled(format!("source: {src}"), app.c(DIM)),
        Span::styled(
            if live { format!("  ● live {}", crate::model::fmt_rel(t_rel)) } else { String::new() },
            app.c(status_color("flight")).add_modifier(Modifier::BOLD),
        ),
    ]));
    let header = lines.len();
    let wrapw = inner.width.saturating_sub(12) as usize;
    let mut rows: Vec<Line> = vec![];
    let mut cur_row = 0usize;
    for (i, e) in events.iter().enumerate() {
        let is_cur = cur == Some(i);
        let past = cur.is_some_and(|c| i < c);
        if is_cur {
            cur_row = rows.len();
        }
        let mark = if is_cur {
            "▶ "
        } else if past {
            "✓ "
        } else {
            "  "
        };
        let st = if is_cur {
            app.c(status_color("go")).add_modifier(Modifier::BOLD)
        } else if past {
            app.c(DIM)
        } else {
            app.c(FG)
        };
        let liftoff = e.offset_secs == 0;
        let maxw = wrapw.max(8);
        let desc: String = if e.description.chars().count() > maxw {
            format!("{}…", e.description.chars().take(maxw - 1).collect::<String>())
        } else {
            e.description.clone()
        };
        rows.push(Line::from(vec![
            Span::styled(mark, st),
            Span::styled(
                format!("{:<10}", crate::model::fmt_rel(e.offset_secs)),
                if liftoff { app.c(ACCENT).add_modifier(Modifier::BOLD) } else { app.c(DIM) },
            ),
            Span::styled(desc, st),
        ]));
        if is_cur {
            if let Some(n) = events.get(i + 1) {
                rows.push(Line::from(Span::styled(format!("            next: {} in {}", n.label, short_dur(n.offset_secs - t_rel)), app.c(DIM))));
            }
        }
    }
    let avail = inner.height.saturating_sub(header as u16 + if plan.as_ref().is_some_and(|p| !p.recovery.is_empty()) { 2 } else { 0 }) as usize;
    let total = rows.len();
    app.plan_max = total.saturating_sub(avail) as u16;
    app.plan_auto = (if cur.is_some() { cur_row.saturating_sub(avail / 3) } else { 0 }).min(total.saturating_sub(avail)) as u16;
    let skip = app.plan_scroll.map(|s| s.min(app.plan_max)).unwrap_or(app.plan_auto) as usize;
    if let Some(s) = &mut app.plan_scroll {
        *s = (*s).min(app.plan_max);
    }
    lines.extend(rows.into_iter().skip(skip).take(avail));
    scrollbar(f, area, total, skip, avail);
    if let Some(p) = &plan {
        if !p.recovery.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![Span::styled("recovery ", app.c(DIM)), Span::styled(p.recovery.clone(), app.c(FG))]));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_telemetry(f: &mut Frame, app: &App, area: Rect) {
    let block = app.block("Telemetry", app.focused(Pane::Telemetry));
    if app.cfg.telemetry_url.is_empty() || app.telemetry.is_empty() {
        let l = app.selected();
        let live = l.map(|l| l.webcast_live || l.status_class == "flight").unwrap_or(false);
        let msg = vec![
            Line::from(Span::styled(if app.cfg.telemetry_url.is_empty() { "no telemetry source configured" } else { "waiting for telemetry…" }, app.c(DIM))),
            Line::raw(""),
            Line::from(Span::styled(if live { "● vehicle/webcast live — press w" } else { "○ not live" }, app.c(if live { Color::Red } else { DIM }))),
            Line::raw(""),
            Line::from(Span::styled("set telemetry_url in config.toml", app.c(DIM))),
        ];
        f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }).block(block), area);
        return;
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let h = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);
    let alt: Vec<u64> = app.telemetry.iter().map(|t| t.0.max(0.0) as u64).collect();
    let vel: Vec<u64> = app.telemetry.iter().map(|t| t.1.max(0.0) as u64).collect();
    let last = app.telemetry.last().copied().unwrap_or_default();
    f.render_widget(Sparkline::default().data(&alt).style(app.c(ACCENT)).block(Block::default().title(format!("alt {:.1} km", last.0))), h[0]);
    f.render_widget(Sparkline::default().data(&vel).style(app.c(Color::Cyan)).block(Block::default().title(format!("vel {:.0} km/h", last.1))), h[1]);
}

/// Compact label for timeline markers ("Starlink Group 15-25" -> "SL 15-25").
pub fn short_mission(l: &Launch, max: usize) -> String {
    let m = if l.mission.is_empty() { l.name.split('|').nth(1).unwrap_or(&l.name).trim().to_string() } else { l.mission.clone() };
    let m = m.replace("Starlink Group ", "SL ").replace("Unknown Payload", &l.short_rocket());
    let m = m.split(" (").next().unwrap_or(&m).to_string();
    if m.chars().count() > max { format!("{}…", m.chars().take(max.saturating_sub(1)).collect::<String>()) } else { m }
}

/// Assign each (x, label width) to the first row where it doesn't overlap
/// (1-cell gap). Returns row index per item, None when all rows are full.
pub fn stagger(items: &[(usize, usize)], rows: usize, width: usize) -> Vec<Option<(usize, usize)>> {
    let mut used: Vec<Vec<(usize, usize)>> = vec![vec![]; rows];
    items
        .iter()
        .map(|&(x, w)| {
            let start = x.min(width.saturating_sub(w));
            for (r, u) in used.iter_mut().enumerate() {
                if u.iter().all(|&(a, b)| start + w + 1 <= a || start >= b + 1) {
                    u.push((start, start + w));
                    return Some((r, start));
                }
            }
            None
        })
        .collect()
}

// ------------------------------------------------------------------ live mission timeline

/// Signed-sqrt time scale: keeps the dense first minutes readable while still
/// fitting an hour-long coast. Returns a column in [0, width-1].
pub fn axis_x(t: f64, t0: f64, t1: f64, width: u16) -> u16 {
    let f = |x: f64| x.signum() * x.abs().sqrt();
    let (a, b) = (f(t0), f(t1));
    if b <= a || width == 0 {
        return 0;
    }
    let u = ((f(t.clamp(t0, t1)) - a) / (b - a)).clamp(0.0, 1.0);
    (u * (width - 1) as f64).round() as u16
}

/// Compact glyph per flight event.
pub fn event_icon(m: &Milestone) -> char {
    let l = format!("{} {}", m.label, m.description).to_lowercase();
    let map: [(&str, char); 14] = [
        ("liftoff", '▲'),
        ("max q", '◈'),
        ("meco", '◼'),
        ("stage sep", '✂'),
        ("stages separate", '✂'),
        ("ses", '»'),
        ("fairing", '◇'),
        ("seco", '■'),
        ("boostback", '↶'),
        ("entry", '↯'),
        ("landing burn", '⇣'),
        ("landing", '⊥'),
        ("deploy", '✦'),
        ("ignition", '✶'),
    ];
    map.iter().find(|(k, _)| l.contains(k)).map(|x| x.1).unwrap_or('•')
}

/// Booster (1st stage) events after stage separation go on their own lane.
pub fn is_booster_event(m: &Milestone, sep: Option<i64>) -> bool {
    let d = m.description.to_lowercase();
    sep.is_some_and(|s| m.offset_secs > s) && (d.starts_with("1st stage") || d.contains("boostback")) && !d.contains("1st and 2nd")
}

fn stage_sep(ev: &[Milestone]) -> Option<i64> {
    ev.iter()
        .find(|e| {
            let d = e.description.to_lowercase();
            d.contains("stages separate") || e.label == "Stage sep"
        })
        .map(|e| e.offset_secs)
}

/// Engines lit at mission time t (for the exhaust flicker).
pub fn burning(ev: &[Milestone], t: f64) -> bool {
    let at = |k: &str| ev.iter().find(|e| e.label.eq_ignore_ascii_case(k)).map(|e| e.offset_secs as f64);
    let spans = [(at("Ignition").or(Some(-3.0)), at("MECO")), (at("SES-1"), at("SECO-1")), (at("SES-2"), at("SECO-2"))];
    spans.iter().any(|(a, b)| matches!((a, b), (Some(a), Some(b)) if t >= *a && t < *b))
}

/// A launch whose mission timeline should replace the 7-day view, with its events.
fn live_mission(app: &App, now: DateTime<Utc>) -> Option<(Launch, Vec<Milestone>)> {
    let ev_for = |l: &Launch| -> Vec<Milestone> {
        match app.plan_for(l) {
            Some(p) if !p.events.is_empty() => p.events.clone(),
            _ => l.milestones.clone(),
        }
    };
    let is_live = |l: &Launch| {
        let ev = ev_for(l);
        let Some(last) = ev.last() else { return false };
        let t = (now - l.net).num_milliseconds() as f64 / 1000.0;
        l.status_id == crate::model::status::IN_FLIGHT || (l.precise() && t >= -600.0 && t <= last.offset_secs as f64 + 60.0)
    };
    let test = app.test.as_ref().and_then(|_| app.cache.launches.iter().find(|l| l.id == testflight::ID));
    let pick = test.or_else(|| app.selected().filter(|l| is_live(l))).or_else(|| app.launches().iter().find(|l| is_live(l)))?;
    if !is_live(pick) {
        return None;
    }
    let ev = ev_for(pick);
    Some((pick.clone(), ev))
}

fn draw_live_timeline(f: &mut Frame, app: &App, area: Rect, inner: Rect, l: &Launch, ev: &[Milestone], now: DateTime<Utc>) {
    let _ = area;
    let t = (now - l.net).num_milliseconds() as f64 / 1000.0;
    let ms = now.timestamp_subsec_millis() as u64 + (now.timestamp() as u64 % 1000) * 1000;
    let pulse = app.reduced_motion || (ms / 300) % 2 == 0;
    let lane_w: u16 = 4;
    let w = inner.width.saturating_sub(lane_w + 1);
    if w < 20 || inner.height < 4 {
        return;
    }
    let t0 = ev.first().map(|e| e.offset_secs as f64).unwrap_or(-600.0).max(-600.0).min(t);
    let t1 = ev.last().map(|e| e.offset_secs as f64).unwrap_or(600.0).max(t0 + 60.0);
    let x0 = inner.x + lane_w;
    let xs = |tt: f64| x0 + axis_x(tt, t0, t1, w);
    let cur = crate::model::current_milestone(ev, t.floor() as i64);
    let next = crate::model::next_milestone(ev, t.floor() as i64);
    let sep = stage_sep(ev);
    let separated = sep.is_some_and(|s| t >= s as f64);
    let (r_head, r_s1, r_s2, r_lab) = (inner.y, inner.y + 1, inner.y + 2, inner.y + 3);
    let buf = f.buffer_mut();
    let st = |c: Color| if app.no_color { Style::default() } else { Style::default().fg(c) };
    // header
    let fl = status_color("flight");
    let mut head = vec![
        Span::styled(if pulse { "● " } else { "○ " }, st(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}  ", if t < 0.0 { "COUNTDOWN" } else { "IN FLIGHT" }), st(fl).add_modifier(Modifier::BOLD)),
        Span::styled(crate::model::fmt_rel(t.floor() as i64), st(FG).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {}", short_mission(l, 28)), st(DIM)),
    ];
    if let Some(c) = cur.and_then(|i| ev.get(i)) {
        head.push(Span::styled("   ▸ ", st(ACCENT)));
        head.push(Span::styled(c.label.clone(), st(status_color("go")).add_modifier(Modifier::BOLD)));
    }
    if let Some(n) = next {
        head.push(Span::styled(format!("   next {} in {}", n.label, short_dur(n.offset_secs - t.floor() as i64)), st(DIM)));
    }
    buf.set_line(inner.x, r_head, &Line::from(head), inner.width);
    // lane labels
    buf.set_string(inner.x, r_s2, if separated { "S2 " } else { "F9 " }, st(DIM));
    if separated {
        buf.set_string(inner.x, r_s1, "S1 ", st(DIM));
    }
    // main track: progress fill + future
    let xr = xs(t);
    for x in x0..x0 + w {
        let (ch, s) = if x < xr { ("━", st(ACCENT)) } else { ("┄", st(BORDER)) };
        buf[(x, r_s2)].set_symbol(ch).set_style(s);
    }
    // booster lane: from separation to landing
    let land = ev.iter().filter(|e| is_booster_event(e, sep)).map(|e| e.offset_secs).max();
    if let (Some(s), Some(lend)) = (sep, land) {
        let (a, b) = (xs(s as f64), xs(lend as f64));
        if separated {
            let xb = xs(t.min(lend as f64));
            for x in a..=b {
                let (ch, sty) = if x < xb { ("━", st(Color::Rgb(0x7a, 0xc7, 0xd6))) } else { ("┄", st(BORDER)) };
                buf[(x, r_s1)].set_symbol(ch).set_style(sty);
            }
            buf[(a, r_s1)].set_symbol("╰").set_style(st(BORDER));
            if t <= lend as f64 + 0.5 {
                buf[(xb, r_s1)].set_symbol("▼").set_style(st(Color::Rgb(0x7a, 0xc7, 0xd6)).add_modifier(Modifier::BOLD));
            }
        }
    }
    // event ticks
    for (i, e) in ev.iter().enumerate() {
        let et = e.offset_secs as f64;
        if et < t0 {
            continue;
        }
        let booster = is_booster_event(e, sep);
        if booster && !separated {
            // show booster events on the main track until separation
        }
        let row = if booster && separated { r_s1 } else { r_s2 };
        let x = xs(et);
        let s = if Some(i) == cur {
            let b = st(status_color("go")).add_modifier(Modifier::BOLD);
            if pulse { b.add_modifier(Modifier::REVERSED) } else { b }
        } else if next.is_some_and(|n| std::ptr::eq(n, e)) {
            st(ACCENT).add_modifier(Modifier::BOLD)
        } else if et <= t {
            st(DIM)
        } else {
            st(FG)
        };
        buf[(x, row)].set_char(event_icon(e)).set_style(s);
    }
    // rocket + flickering exhaust
    if t >= -3.0 && t <= t1 {
        if burning(ev, t) && !app.reduced_motion {
            let flames = ["≋", "∿", "~"];
            let cols = [Color::Rgb(0xff, 0xd2, 0x4a), Color::Rgb(0xf2, 0x9e, 0x4c), Color::Rgb(0xe8, 0x6a, 0x6a)];
            for k in 1..=2u16 {
                if xr >= x0 + k {
                    let idx = ((ms / 90) as usize + k as usize) % 3;
                    buf[(xr - k, r_s2)].set_symbol(flames[idx]).set_style(st(cols[(idx + k as usize) % 3]).add_modifier(Modifier::BOLD));
                }
            }
        }
        let rocket = if app.cfg.tui.nerd_font { "\u{f135}" } else { "➤" };
        buf[(xr, r_s2)].set_symbol(rocket).set_style(st(FG).add_modifier(Modifier::BOLD));
    } else if t < -3.0 {
        buf[(x0, r_s2)].set_symbol("▲").set_style(st(FG).add_modifier(Modifier::BOLD));
    }
    // labels: current + next only, staggered over the remaining rows
    let mut labels: Vec<(u16, String, Style)> = vec![];
    if let Some(c) = cur.and_then(|i| ev.get(i)) {
        if c.offset_secs as f64 >= t0 {
            labels.push((xs(c.offset_secs as f64), format!("{} {}", event_icon(c), c.label), st(status_color("go")).add_modifier(Modifier::BOLD)));
        }
    }
    if let Some(n) = next {
        labels.push((xs(n.offset_secs as f64), format!("{} {} {}", event_icon(n), n.label, crate::model::fmt_rel(n.offset_secs)), st(ACCENT)));
    }
    let rows = inner.y + inner.height - r_lab;
    let items: Vec<(usize, usize)> = labels.iter().map(|(x, s, _)| ((*x - inner.x) as usize, s.chars().count())).collect();
    for ((_, s, sty), p) in labels.iter().zip(stagger(&items, rows as usize, inner.width as usize)) {
        if let Some((row, start)) = p {
            buf.set_string(inner.x + start as u16, r_lab + row as u16, s, *sty);
        }
    }
}

fn draw_timeline(f: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    if let Some((l, ev)) = live_mission(app, now) {
        let title = format!("Mission Timeline · {}", if l.id == testflight::ID { "TEST FLIGHT" } else { "LIVE" });
        let block = app.block(&title, false).border_style(app.c(status_color("flight")));
        let inner = block.inner(area);
        f.render_widget(block, area);
        draw_live_timeline(f, app, area, inner, &l, &ev, now);
        return;
    }
    let block = app.block("Timeline · next 7 days", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 10 || inner.height < 3 {
        return;
    }
    let w = inner.width as i64;
    let span = Duration::days(7).num_seconds();
    let xpos = |t: DateTime<Utc>| -> Option<i64> {
        let s = (t - now).num_seconds();
        if s < 0 || s > span { None } else { Some(s * (w - 1) / span) }
    };
    let mut labels = vec![' '; w as usize];
    let mut axis: Vec<(String, Style)> = vec![("─".into(), app.c(BORDER)); w as usize];
    let start_local = now.with_timezone(&Local);
    for d in 0..=7 {
        let day = (start_local.date_naive() + chrono::Days::new(d)).and_hms_opt(0, 0, 0).unwrap();
        let Some(t) = day.and_local_timezone(Local).single() else { continue };
        if let Some(x) = xpos(t.with_timezone(&Utc)) {
            axis[x as usize] = ("┼".into(), app.c(BORDER));
            let txt = t.format("%a %-d").to_string();
            for (i, ch) in txt.chars().enumerate() {
                if let Some(c) = labels.get_mut(x as usize + 1 + i) {
                    *c = ch;
                }
            }
        }
    }
    axis[0] = ("▶".into(), app.c(ACCENT));
    let sel = app.selected().map(|l| l.id.clone());
    let nrows = inner.height.saturating_sub(2) as usize;
    // selected first so it always gets a label, then chronological
    let mut ls: Vec<(&Launch, i64)> = app.launches().iter().filter_map(|l| Some((l, xpos(l.net)?))).collect();
    ls.sort_by_key(|(l, x)| (Some(&l.id) != sel.as_ref(), *x));
    let names: Vec<String> = ls.iter().map(|(l, _)| format!("•{}", short_mission(l, 14))).collect();
    let items: Vec<(usize, usize)> = ls.iter().zip(&names).map(|((_, x), n)| (*x as usize, n.chars().count())).collect();
    let mut placed = stagger(&items, nrows, w as usize);
    // fall back to a tiny "•" when no room for the name
    let mut grid: Vec<Vec<(String, Style)>> = vec![vec![(" ".into(), Style::default()); w as usize]; nrows];
    for (i, ((l, x), name)) in ls.iter().zip(&names).enumerate() {
        let is_sel = Some(&l.id) == sel.as_ref();
        let st = app.c(status_color(&l.status_class));
        axis[*x as usize] = (if is_sel { "▲" } else { "┴" }.into(), if is_sel { st.add_modifier(Modifier::BOLD) } else { st });
        let Some((row, start)) = placed[i].take() else { continue };
        let lst = if is_sel {
            if app.no_color {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Color::Black).bg(status_color(&l.status_class)).add_modifier(Modifier::BOLD)
            }
        } else {
            app.c(FG)
        };
        for (k, ch) in name.chars().enumerate() {
            let s = if k == 0 { st.add_modifier(Modifier::BOLD) } else { lst };
            grid[row][start + k] = (ch.to_string(), s);
        }
    }
    let to_line = |v: Vec<(String, Style)>| Line::from(v.into_iter().map(|(c, s)| Span::styled(c, s)).collect::<Vec<_>>());
    let mut lines = vec![Line::from(Span::styled(labels.into_iter().collect::<String>(), app.c(DIM))), to_line(axis)];
    lines.extend(grid.into_iter().map(to_line));
    f.render_widget(Paragraph::new(lines), inner);
}

fn ago(t: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    match t {
        None => "never".into(),
        Some(t) => {
            let m = (now - t).num_minutes();
            if m < 1 {
                "just now".into()
            } else if m < 60 {
                format!("{m}m ago")
            } else {
                format!("{}h{}m ago", m / 60, m % 60)
            }
        }
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect, now: DateTime<Utc>) {
    let key = |k: &str, d: &str| vec![Span::styled(format!(" {k}"), app.c(ACCENT).add_modifier(Modifier::BOLD)), Span::styled(format!(" {d} "), app.c(DIM))];
    let mut spans = vec![];
    for (k, d) in [
        ("tab/h/l", "focus"),
        ("j/k", "scroll"),
        ("w", "webcast"),
        ("n", "news"),
        ("f", "flight plan"),
        ("s", "star"),
        ("r", "refresh"),
        ("t", "telemetry"),
        ("?", "help"),
        ("q", "quit"),
    ] {
        spans.extend(key(k, d));
    }
    let local_left = app.state.budget.remaining(now, app.cfg.max_calls_per_hour);
    let thr = &app.cache.throttle;
    let sy = &app.cache.sync;
    let mut sync = String::new();
    if let Some(t) = sy.last_at {
        sync.push_str(&format!("synced {} {} ", sy.last_label.split(" · ").next().unwrap_or(""), ago(Some(t), now)));
        if let Some(d) = sy.last_net_shift_secs {
            sync.push_str(&format!("(NET {d:+}s) "));
        }
    }
    if let Some(t) = sy.next_at {
        let d = (t - now).num_seconds();
        sync.push_str(&format!("· next {} in {} ", sy.next_label.split(" · ").next().unwrap_or(""), short_dur(d.max(0))));
    }
    if sy.reserved > 0 {
        sync.push_str(&format!("({} reserved) ", sy.reserved));
    }
    if sy.clock_offset_ms != 0 {
        sync.push_str(&format!("· clock {:+.1}s ", sy.clock_offset_ms as f64 / 1000.0));
    }
    if let Some(l) = app.nudge_target() {
        if let Some(o) = app.orig_nets.get(&l.id).and_then(|n| app.nudge.applies(&Launch { net: *n, ..l.clone() }).map(|t0| (t0 - *n).num_seconds())) {
            sync.push_str(&format!("· T-0 nudge {o:+}s "));
        }
    }
    if !sync.is_empty() {
        sync.push_str("· ");
    }
    let mut right = format!(
        "{sync}cache {} · LL2 {}/{} left (local {}/{}) ",
        ago(app.cache.fetched_at, now),
        thr.remaining(),
        if thr.limit > 0 { thr.limit } else { 15 },
        local_left,
        app.cfg.max_calls_per_hour
    );
    if app.fetching {
        right = format!("⟳ fetching… · {right}");
    }
    if let Some(e) = &app.cache.last_error {
        right = format!("⚠ {} · {right}", e.chars().take(40).collect::<String>());
    }
    if let Some((t, at)) = &app.toast {
        if at.elapsed().as_secs() < 4 {
            right = format!("{t} · {right}");
        }
    }
    let h = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Min(10), Constraint::Length(right.chars().count() as u16)]).split(area);
    f.render_widget(Paragraph::new(Line::from(spans)), h[0]);
    f.render_widget(Paragraph::new(Span::styled(right, app.c(DIM))).alignment(Alignment::Right), h[1]);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let r = centered(area, 64, 31);
    f.render_widget(Clear, r);
    let rows = [
        ("Tab / S-Tab", "focus next / previous pane"),
        ("h / l  H / L", "focus previous / next pane"),
        ("j / k / ↑ / ↓", "scroll focused pane (Upcoming: select)"),
        ("g / G", "top / bottom of focused pane"),
        ("PgDn/PgUp C-d/C-u", "page down / up"),
        ("mouse", "click focuses/selects, wheel scrolls pane"),
        ("w", "open webcast (or info page)"),
        ("n", "news for selected launch (Enter opens)"),
        ("s", "star/unstar provider (T-24h & T-1h alerts)"),
        ("r", "refresh (asks daemon; respects 15 req/hr)"),
        ("t", "toggle telemetry pane"),
        ("f", "toggle flight plan pane (SpaceX timeline)"),
        ("p", "open spacex.com mission page"),
        ("[ / ]  J / K", "scroll details from any pane"),
        ("< / >", "nudge T-0 −1 s / +1 s (sync to webcast)"),
        ("M", "mark liftoff now (T-0 = this instant)"),
        ("=", "clear T-0 nudge"),
        ("?", "this help"),
        ("q / Esc", "quit / close overlay"),
    ];
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:>16}  "), app.c(ACCENT).add_modifier(Modifier::BOLD)), Span::styled(*d, app.c(FG))]))
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("colors: go · tbc · tbd · hold · in flight · success · failure", app.c(DIM))));
    let legend: Vec<Span> =
        ["go", "tbc", "tbd", "hold", "flight", "success", "failure"].iter().map(|c| Span::styled(format!(" ■ {c} "), app.c(status_color(c)))).collect();
    lines.push(Line::from(legend));
    lines.push(Line::from(Span::styled(format!("config: {}", paths::config_file().display()), app.c(DIM))));
    f.render_widget(Paragraph::new(lines).block(app.block("Help", true)), r);
}

fn draw_news(f: &mut Frame, app: &App, area: Rect) {
    let r = centered(area, (area.width * 3 / 4).max(60), (area.height * 3 / 4).max(12));
    f.render_widget(Clear, r);
    let l = app.selected();
    let title = l.map(|l| format!("News · {}", l.name)).unwrap_or_else(|| "News".into());
    let block = app.block(&title, true);
    let items = l.and_then(|l| app.news.get(&l.id));
    let lines: Vec<Line> = match items {
        None => vec![Line::from(Span::styled("loading from Spaceflight News API…", app.c(DIM)))],
        Some(v) if v.is_empty() => vec![Line::from(Span::styled("no articles yet", app.c(DIM)))],
        Some(v) => v
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                let sel = i == app.news_sel;
                let st = if sel { app.c(ACCENT).add_modifier(Modifier::BOLD) } else { app.c(FG) };
                vec![
                    Line::from(vec![Span::styled(if sel { "▌ " } else { "  " }, app.c(ACCENT)), Span::styled(a.title.clone(), st)]),
                    Line::from(Span::styled(
                        format!(
                            "  {} · {}",
                            a.news_site,
                            a.published_at.map(|t| t.with_timezone(&Local).format("%b %-d %H:%M").to_string()).unwrap_or_default()
                        ),
                        app.c(DIM),
                    )),
                    if sel { Line::from(Span::styled(format!("  {}", a.summary), app.c(DIM))) } else { Line::raw("") },
                ]
            })
            .collect(),
    };
    let inner = block.inner(r);
    let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    let total = para.line_count(inner.width);
    let vis = inner.height as usize;
    // keep the selected article (3 lines each, selected one wraps longer) in view
    let pos = (app.news_sel * 3).saturating_sub(vis / 3).min(total.saturating_sub(vis));
    f.render_widget(block, r);
    f.render_widget(para.scroll((pos as u16, 0)), inner);
    scrollbar(f, r, total, pos, vis);
}

// ------------------------------------------------------------------ loop

/// Cell updates between two frames with correct coordinates for any size.
///
/// ratatui 0.29's `Buffer::diff` maps a cell index to (x, y) through `i as u16`, so on
/// terminals with more than 65,535 cells (e.g. 540x129, a maximized 4K Ghostty) every
/// cell past that index is sent to the wrong place: the bottom panes land on the top
/// rows and the frame scrambles. Same algorithm as ratatui (incl. wide-char handling),
/// with usize index math.
fn frame_diff<'a>(prev: &ratatui::buffer::Buffer, next: &'a ratatui::buffer::Buffer) -> Vec<(u16, u16, &'a ratatui::buffer::Cell)> {
    use unicode_width::UnicodeWidthStr;
    let a = next.area;
    let w = a.width.max(1) as usize;
    let mut out = vec![];
    let (mut invalidated, mut to_skip) = (0usize, 0usize);
    for (i, (cur, old)) in next.content.iter().zip(prev.content.iter()).enumerate() {
        if !cur.skip && (cur != old || invalidated > 0) && to_skip == 0 {
            out.push((a.x + (i % w) as u16, a.y + (i / w) as u16, cur));
        }
        to_skip = cur.symbol().width().saturating_sub(1);
        invalidated = cur.symbol().width().max(old.symbol().width()).max(invalidated).saturating_sub(1);
    }
    out
}

/// `Terminal::draw` with our own diff (see `frame_diff`); `prev` is the last frame sent.
fn draw_frame<B: ratatui::backend::Backend>(
    term: &mut ratatui::Terminal<B>,
    prev: &mut Option<ratatui::buffer::Buffer>,
    render: impl FnOnce(&mut Frame),
) -> std::io::Result<()> {
    let before = term.size()?;
    term.autoresize()?; // clears the screen on resize
    if term.size()? != before {
        *prev = None;
    }
    render(&mut term.get_frame());
    let next = term.current_buffer_mut().clone();
    let base = match prev.take() {
        Some(b) if b.area == next.area => b,
        _ => ratatui::buffer::Buffer::empty(next.area),
    };
    let updates = frame_diff(&base, &next);
    term.backend_mut().draw(updates.into_iter())?;
    term.hide_cursor()?; // the TUI never places a cursor
    term.swap_buffers();
    ratatui::backend::Backend::flush(term.backend_mut())?;
    *prev = Some(next);
    Ok(())
}

pub fn run() -> Result<()> {
    let cfg = Config::load()?;
    let (tx, rx) = channel();
    let mut app = App {
        cfg,
        cache: Cache::default(),
        state: State::default(),
        mtime: None,
        table: TableState::default(),
        overlay: Overlay::None,
        news_sel: 0,
        news: Default::default(),
        news_loading: None,
        telemetry: vec![],
        toast: None,
        tick: 0,
        list_area: Rect::default(),
        tx,
        rx,
        fetching: false,
        no_color: std::env::var_os("NO_COLOR").is_some(),
        reduced_motion: std::env::var("LIFTOFF_REDUCED_MOTION").is_ok_and(|v| v == "1"),
        quit: false,
        patches: Default::default(),
        patch_loading: Default::default(),
        plans: Default::default(),
        plan_loading: Default::default(),
        details_scroll: 0,
        side: SidePane::FlightPlan,
        focus: Pane::List,
        visible: vec![],
        details_max: 0,
        plan_scroll: None,
        plan_max: 0,
        plan_auto: 0,
        test: None,
        chord: None,
        sel_id: None,
        nudge: crate::nudge::load(),
        orig_nets: Default::default(),
    };
    app.reload_if_changed();
    if app.cache.launches.is_empty() {
        app.spawn_fetch();
    }
    if !app.cfg.telemetry_url.is_empty() {
        let url = app.cfg.telemetry_url.clone();
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            loop {
                // Expected JSON: {"altitude": km, "velocity": km/h} (Launch Dashboard-style relay).
                let v = ureq::get(&url).timeout(std::time::Duration::from_secs(5)).call().ok().and_then(|r| r.into_json::<serde_json::Value>().ok());
                let p = v.and_then(|v| Some((v["altitude"].as_f64()?, v["velocity"].as_f64()?)));
                if tx.send(Msg::Telemetry(p)).is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }

    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut last_reload = Instant::now();
    let mut prev_frame: Option<ratatui::buffer::Buffer> = None;
    let res = (|| -> Result<()> {
        while !app.quit {
            app.update_test(Utc::now());
            draw_frame(&mut terminal, &mut prev_frame, |f| draw(f, &mut app))?;
            // faster frames while a mission timeline is animating
            let live = app.test.is_some() || app.launches().iter().any(|l| l.status_id == crate::model::status::IN_FLIGHT);
            // wake right after each whole (corrected) second so the countdown flips on time, with the bar
            let frame_ms = if live { 60 } else { 125 };
            if event::poll(std::time::Duration::from_millis(frame_ms.min(schedule::ms_to_next_second(app.now()))))? {
                match event::read()? {
                    Event::Key(k) => app.on_key(k),
                    Event::Mouse(m) => app.on_mouse(m),
                    _ => {}
                }
            }
            while let Ok(m) = app.rx.try_recv() {
                match m {
                    Msg::News(id, a) => {
                        app.news.insert(id, a);
                        app.news_loading = None;
                    }
                    Msg::Fetched(r) => {
                        app.fetching = false;
                        app.mtime = None;
                        match r {
                            Err(e) => app.toast(format!("fetch failed: {e}")),
                            Ok(c) if c.sync.note_at.is_some_and(|t| Utc::now() - t < Duration::seconds(5)) => app.toast(c.sync.note.clone()),
                            Ok(_) => {}
                        }
                    }
                    Msg::Telemetry(Some(p)) => {
                        app.telemetry.push(p);
                        if app.telemetry.len() > 300 {
                            app.telemetry.remove(0);
                        }
                    }
                    Msg::Telemetry(None) => {}
                    Msg::Patch(u, img) => {
                        app.patch_loading.remove(&u);
                        app.patches.insert(u, img);
                    }
                    Msg::Plan(id, p) => {
                        app.plan_loading.remove(&id);
                        app.plans.insert(id, p);
                    }
                }
            }
            if last_reload.elapsed().as_secs() >= 1 {
                app.reload_if_changed();
                last_reload = Instant::now();
            }
            app.update_test(Utc::now());
            app.tick += 1;
        }
        Ok(())
    })();
    if app.test.is_some() {
        testflight::clear_marker();
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn test_app(cache: Cache) -> App {
        let (tx, rx) = channel();
        let mut cfg = Config::default();
        cfg.spacex_flight_plans = false; // no network in tests
        cfg.tui.mission_patch = false;
        let mut app = App {
            cfg,
            cache,
            state: State::default(),
            mtime: None,
            table: TableState::default(),
            overlay: Overlay::None,
            news_sel: 0,
            news: Default::default(),
            news_loading: None,
            telemetry: vec![],
            toast: None,
            tick: 0,
            list_area: Rect::default(),
            tx,
            rx,
            fetching: false,
            no_color: false,
            reduced_motion: true,
            quit: false,
            patches: Default::default(),
            patch_loading: Default::default(),
            plans: Default::default(),
            plan_loading: Default::default(),
            details_scroll: 0,
            side: SidePane::FlightPlan,
            focus: Pane::List,
            visible: vec![],
            details_max: 0,
            plan_scroll: None,
            plan_max: 0,
            plan_auto: 0,
            test: None,
            chord: None,
            sel_id: None,
            nudge: Default::default(),
            orig_nets: Default::default(),
        };
        app.cache.launches.iter_mut().for_each(sanitize);
        app.table.select(Some(1));
        app
    }

    /// Regression: >65,535 cells (maximized 4K Ghostty, 540x129) scrambled the frame.
    #[test]
    fn huge_terminal_frame_lands_where_rendered() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        for (w, h) in [(120u16, 40u16), (200, 50), (320, 80), (400, 100), (540, 129), (600, 160)] {
            let mut app = test_app(Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() });
            let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut prev = None;
            for _ in 0..3 {
                draw_frame(&mut term, &mut prev, |f| draw(f, &mut app)).unwrap();
                app.tick += 1;
            }
            // what reached the "screen" equals what was rendered, cell for cell
            let shown = term.backend().buffer().clone();
            let row = |b: &ratatui::buffer::Buffer, y: u16| (0..w).map(|x| b[(x, y)].symbol().to_string()).collect::<String>();
            assert!(row(&shown, 0).starts_with("╭ Next Launch"), "{w}x{h}: top row {:?}", &row(&shown, 0)[..40]);
            assert!(row(&shown, h - 8).starts_with("╭ Timeline"), "{w}x{h}: timeline row");
            assert!(row(&shown, h - 1).starts_with(" tab/h/l focus"), "{w}x{h}: footer");
        }
    }

    #[test]
    #[ignore]
    fn dump_sizes() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        for (w, h) in [(120u16, 40u16), (200, 50), (320, 80), (400, 100), (540, 129)] {
            let mut app = test_app(Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() });
            let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
            draw_frame(&mut term, &mut None, |f| draw(f, &mut app)).unwrap();
            let buf = term.backend().buffer().clone();
            let mut out = String::new();
            for y in 0..h {
                for x in 0..w {
                    out.push_str(buf[(x, y)].symbol());
                }
                out.push('\n');
            }
            std::fs::write(format!("/tmp/tui-{w}x{h}.txt"), out).unwrap();
        }
    }

    #[test]
    fn full_frame_has_no_control_chars_and_no_bleed() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        let mut launches = api::parse_upcoming(&v).unwrap();
        // simulate an old cache written before parse-time cleaning
        launches[1].mission_description = "x\r\nCargo Dragon 2 brings supplies\tand payloads".into();
        let cache = Cache { launches, ..Default::default() };
        let mut app = test_app(cache);
        let mut term = ratatui::Terminal::new(TestBackend::new(200, 60)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        for c in buf.content() {
            assert!(!c.symbol().chars().any(|ch| ch.is_control()), "control char in cell: {:?}", c.symbol());
        }
        // description only appears inside the right column (details pane)
        let left_w = 200 * 44 / 100;
        for y in 0..60u16 {
            let row: String = (0..left_w).map(|x| buf[(x, y)].symbol().to_string()).collect();
            assert!(!row.contains("brings supplies"), "bleed on row {y}: {row}");
        }
        // every cell got the solid background
        assert!(buf.content().iter().all(|c| c.bg != Color::Reset));
    }

    #[test]
    fn focus_cycles_and_skips_hidden() {
        use Pane::*;
        let all = [List, Details, Plan, Map];
        assert_eq!(next_focus(&all, List, 1), Details);
        assert_eq!(next_focus(&all, Map, 1), List);
        assert_eq!(next_focus(&all, List, -1), Map);
        let no_plan = [List, Details, Map];
        assert_eq!(next_focus(&no_plan, Details, 1), Map);
        assert_eq!(next_focus(&no_plan, Plan, 1), List);
        assert_eq!(next_focus(&[], Map, 1), Map);
    }

    fn press(app: &mut App, term: &mut ratatui::Terminal<TestBackend>, keys: &str) {
        for c in keys.chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            term.draw(|f| draw(f, app)).unwrap();
        }
    }
    fn side_app() -> (App, ratatui::Terminal<TestBackend>) {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        let mut app = test_app(Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() });
        app.side = SidePane::Hidden;
        let mut term = ratatui::Terminal::new(TestBackend::new(200, 50)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        (app, term)
    }
    fn order(app: &App) -> Vec<Pane> {
        app.visible.iter().map(|v| v.0).collect()
    }

    #[test]
    fn side_pane_toggle_orderings() {
        use Pane::*;
        let cases: [(&str, SidePane, Vec<Pane>); 5] = [
            ("ftf", SidePane::FlightPlan, vec![List, Details, Plan, Map]),
            ("tf", SidePane::FlightPlan, vec![List, Details, Plan, Map]),
            ("ff", SidePane::Hidden, vec![List, Details, Map]),
            ("tt", SidePane::Hidden, vec![List, Details, Map]),
            ("ft", SidePane::Telemetry, vec![List, Details, Telemetry, Map]),
        ];
        for (keys, want, panes) in cases {
            let (mut app, mut term) = side_app();
            press(&mut app, &mut term, keys);
            assert_eq!(app.side, want, "keys {keys}");
            assert_eq!(order(&app), panes, "keys {keys}");
            let all: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
            assert_eq!(all.contains("Flight Plan"), want == SidePane::FlightPlan, "keys {keys}");
            assert_eq!(all.contains("Telemetry "), want == SidePane::Telemetry, "keys {keys}");
        }
    }

    #[test]
    fn focus_follows_side_pane() {
        let (mut app, mut term) = side_app();
        press(&mut app, &mut term, "t");
        app.focus = Pane::Telemetry;
        press(&mut app, &mut term, "f"); // telemetry replaced by flight plan
        assert_eq!(app.focus, Pane::List, "focus falls back when its pane disappears");
        press(&mut app, &mut term, "ll");
        assert_eq!(app.focus, Pane::Plan);
        press(&mut app, &mut term, "t");
        app.focus = Pane::Details;
        press(&mut app, &mut term, "l");
        assert_eq!(app.focus, Pane::Telemetry);
        press(&mut app, &mut term, "l");
        assert_eq!(app.focus, Pane::Map);
    }

    #[test]
    fn axis_positions() {
        assert_eq!(axis_x(-600.0, -600.0, 3849.0, 101), 0);
        assert_eq!(axis_x(3849.0, -600.0, 3849.0, 101), 100);
        assert_eq!(axis_x(99999.0, -600.0, 3849.0, 101), 100);
        let xs: Vec<u16> = [-600.0, -60.0, 0.0, 72.0, 148.0, 501.0, 3849.0].iter().map(|t| axis_x(*t, -600.0, 3849.0, 101)).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        // sqrt scale gives the first 10 min of flight more room than a linear one
        assert!(axis_x(600.0, -600.0, 3849.0, 101) > (101u32 * 1200 / 4449) as u16);
        let ev = testflight::events();
        assert!(burning(&ev, 10.0) && !burning(&ev, 150.0) && burning(&ev, 300.0) && !burning(&ev, 2000.0));
        let sep = stage_sep(&ev);
        assert_eq!(sep, Some(148));
        let booster: Vec<&str> = ev.iter().filter(|e| is_booster_event(e, sep)).map(|e| e.label.as_str()).collect();
        assert_eq!(booster, vec!["Entry burn", "Entry end", "Landing burn", "Landing"]);
        assert_eq!(event_icon(&ev[11]), '◼'); // MECO
    }

    #[test]
    fn test_flight_chord_and_live_timeline() {
        let (mut app, mut term) = side_app();
        let n = app.cache.launches.len();
        // 't' alone toggles telemetry as usual; chord must be Ctrl-x t f
        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        press(&mut app, &mut term, "tf");
        assert!(app.test.is_some());
        assert_eq!(app.side, SidePane::FlightPlan);
        assert_eq!(app.cache.launches.len(), n + 1);
        assert_eq!(app.selected().unwrap().id, testflight::ID);
        // jump to T+2:30 (after stage sep): shift start back
        app.test.as_mut().unwrap().started -= std::time::Duration::from_secs(270);
        app.update_test(Utc::now());
        term.draw(|f| draw(f, &mut app)).unwrap();
        let all: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(all.contains("Mission Timeline · TEST FLIGHT"));
        assert!(all.contains("IN FLIGHT") && all.contains("S1 ") && all.contains("S2 "));
        assert!(all.contains("Stage sep"));
        // never persisted
        assert!(!paths::cache_file().exists() || !std::fs::read_to_string(paths::cache_file()).unwrap_or_default().contains(testflight::ID));
        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        press(&mut app, &mut term, "tf");
        assert!(app.test.is_none());
        assert_eq!(app.cache.launches.len(), n);
        // a stray 'f' after an expired/other chord just toggles the side pane
        press(&mut app, &mut term, "f");
        assert_eq!(app.side, SidePane::Hidden);
    }

    #[test]
    fn selection_survives_reload_and_loop_restart() {
        let (mut app, mut term) = side_app();
        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        press(&mut app, &mut term, "tf");
        assert_eq!(app.selected().unwrap().id, testflight::ID);
        // > 5 s of frames with periodic cache reloads (the daemon rewrites the file)
        let disk = {
            let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
            Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() }
        };
        for i in 0..60 {
            app.test.as_mut().unwrap().started -= std::time::Duration::from_millis(125);
            app.update_test(Utc::now());
            if i % 8 == 0 {
                app.apply_cache(disk.clone());
            }
            term.draw(|f| draw(f, &mut app)).unwrap();
            assert_eq!(app.selected().unwrap().id, testflight::ID, "frame {i}");
        }
        // loop restart
        app.test.as_mut().unwrap().started -= std::time::Duration::from_secs_f64(testflight::period());
        app.update_test(Utc::now());
        app.apply_cache(disk.clone());
        assert_eq!(app.selected().unwrap().id, testflight::ID);
        assert_eq!(app.cache.launches.iter().filter(|l| l.id == testflight::ID).count(), 1);
        // a real launch stays selected by id when the list is re-sorted on reload
        press(&mut app, &mut term, "jj");
        let id = app.selected().unwrap().id.clone();
        let mut shuffled = disk.clone();
        shuffled.launches.reverse();
        app.apply_cache(shuffled);
        assert_eq!(app.selected().unwrap().id, id);
        // selected id disappears -> index 0
        let mut gone = disk.clone();
        gone.launches.retain(|l| l.id != id);
        app.apply_cache(gone);
        assert_eq!(app.table.selected(), Some(0));
    }

    #[test]
    fn nudge_keys_reanchor_selected_launch() {
        let (mut app, mut term) = side_app();
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        app.apply_cache(Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() });
        let id = app.selected().unwrap().id.clone();
        let orig = app.orig_nets[&id];
        press(&mut app, &mut term, ">>><");
        assert_eq!(app.selected().unwrap().net, orig + Duration::seconds(2));
        // a reload (LL2 revised NET by 1 s at the T-5s sync) keeps the absolute nudged T-0
        let mut c = Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() };
        c.launches.iter_mut().find(|l| l.id == id).unwrap().net = orig + Duration::seconds(1);
        app.apply_cache(c);
        assert_eq!(app.selected().unwrap().net, orig + Duration::seconds(2));
        press(&mut app, &mut term, "=");
        assert_eq!(app.selected().unwrap().net, orig + Duration::seconds(1), "cleared -> LL2 NET");
        press(&mut app, &mut term, "M");
        assert!((app.selected().unwrap().net - Utc::now()).num_seconds().abs() <= 1);
    }

    #[test]
    fn scroll_clamps() {
        assert_eq!(clamp_scroll(0, -5, 10), 0);
        assert_eq!(clamp_scroll(8, 5, 10), 10);
        assert_eq!(clamp_scroll(3, 2, 10), 5);
        assert_eq!(clamp_scroll(3, 2, 0), 0);
    }

    #[test]
    fn keys_drive_focus_and_scroll() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        let cache = Cache { launches: api::parse_upcoming(&v).unwrap(), ..Default::default() };
        let mut app = test_app(cache);
        app.table.select(Some(0));
        let mut term = ratatui::Terminal::new(TestBackend::new(200, 50)).unwrap();
        let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
        term.draw(|f| draw(f, &mut app)).unwrap();
        let order: Vec<Pane> = app.visible.iter().map(|v| v.0).collect();
        assert_eq!(order, vec![Pane::List, Pane::Details, Pane::Plan, Pane::Map]);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Pane::Details);
        assert!(app.details_max > 0, "details should overflow at this size");
        for _ in 0..500 {
            key(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.details_scroll, app.details_max);
        assert_eq!(app.table.selected(), Some(0), "j scrolls details, not the list");
        key(&mut app, KeyCode::Char('g'));
        assert_eq!(app.details_scroll, 0);
        key(&mut app, KeyCode::Char('l'));
        assert_eq!(app.focus, Pane::Plan);
        key(&mut app, KeyCode::Char('G'));
        assert_eq!(app.plan_scroll, Some(app.plan_max));
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::Char('h'));
        assert_eq!(app.focus, Pane::List);
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.table.selected(), Some(1));
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let all: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(all.contains("▸ Upcoming"), "focused title marker");
        assert!(all.contains('┃'), "scrollbar thumb drawn on an overflowing pane");
    }

    #[test]
    fn stagger_avoids_overlap() {
        let p = stagger(&[(0, 8), (3, 8), (5, 8), (20, 5), (100, 10)], 2, 60);
        assert_eq!(p[0], Some((0, 0)));
        assert_eq!(p[1], Some((1, 3)));
        assert_eq!(p[2], None); // both rows taken around x=5
        assert_eq!(p[3], Some((0, 20)));
        assert_eq!(p[4], Some((0, 50))); // clamped to fit width
    }

    #[test]
    fn map_labels_do_not_collide() {
        let area = Rect { x: 0, y: 0, width: 80, height: 20 };
        let mut taken = vec![];
        let spots = place_labels(&[(10, 5, 8), (11, 5, 8), (12, 5, 8), (79, 19, 6)], area, &mut taken);
        let rects: Vec<Rect> = spots.iter().zip([8u16, 8, 8, 6]).filter_map(|(s, w)| s.map(|(x, y)| Rect { x, y, width: w, height: 1 })).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.x + a.width <= 80 && a.y < 20);
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b));
            }
        }
        assert!(spots[3].is_some());
    }

    #[test]
    fn labels_and_bars() {
        assert_eq!(pad_label("Space Launch Complex 40", 3), "SLC-40 ×3");
        assert_eq!(pad_label("Launch Complex 39A", 1), "LC-39A");
        let l = Launch { mission: "Starlink Group 15-25".into(), ..Default::default() };
        assert_eq!(short_mission(&l, 14), "SL 15-25");
        let l = Launch { mission: "Bandwagon 5 (Dedicated Mid-Inclination Rideshare)".into(), ..Default::default() };
        assert_eq!(short_mission(&l, 14), "Bandwagon 5");
        let l = Launch { mission: "Martian Moons eXploration".into(), ..Default::default() };
        assert_eq!(short_mission(&l, 10).chars().count(), 10);
        let t0 = Utc::now();
        let b = window_bar(t0, t0 + Duration::hours(4), t0 + Duration::hours(2), t0 - Duration::hours(1), 21);
        assert_eq!(b.chars().count(), 21);
        assert_eq!(b.chars().nth(10), Some('▲'));
        assert_eq!(parse_hex("#ff0080"), Some(Color::Rgb(255, 0, 128)));
        assert_eq!(parse_hex("nope"), None);
    }
    #[test]
    fn big_text_is_rectangular() {
        let r = big_text("T-02:14:07", false);
        assert_eq!(r.len(), 5);
        let w = r[0].chars().count();
        assert!(r.iter().all(|x| x.chars().count() == w));
    }
    #[test]
    fn short_countdown_format() {
        let now = Utc::now();
        let mut l = Launch { net: now + Duration::seconds(3725), net_precision: "MIN".into(), ..Default::default() };
        assert_eq!(short_countdown(&l, now), "-01:02:05");
        l.net = now + Duration::days(2) + Duration::hours(3);
        assert_eq!(short_countdown(&l, now), "-2d03h");
    }
}
