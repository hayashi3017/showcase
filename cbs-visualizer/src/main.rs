use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Vec2};

// ── domain types ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Pos {
    x: i32,
    y: i32,
}

impl Pos {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    fn manhattan(self, other: Self) -> usize {
        ((self.x - other.x).abs() + (self.y - other.y).abs()) as usize
    }
}

#[derive(Clone, Debug)]
struct Grid {
    width: i32,
    height: i32,
    walls: HashSet<Pos>,
}

impl Grid {
    fn new(width: i32, height: i32, walls: &[(i32, i32)]) -> Self {
        Self {
            width,
            height,
            walls: walls.iter().map(|&(x, y)| Pos::new(x, y)).collect(),
        }
    }
    fn in_bounds(&self, p: Pos) -> bool {
        p.x >= 0 && p.x < self.width && p.y >= 0 && p.y < self.height
    }
    fn valid(&self, p: Pos) -> bool {
        self.in_bounds(p) && !self.walls.contains(&p)
    }
    fn neighbors_with_wait(&self, p: Pos) -> Vec<Pos> {
        [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .map(|&(dx, dy)| Pos::new(p.x + dx, p.y + dy))
            .filter(|&next| self.valid(next))
            .collect()
    }
}

#[derive(Clone, Debug)]
struct Agent {
    name: String,
    start: Pos,
    goal: Pos,
    color: Color32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Constraint {
    Vertex { agent: usize, pos: Pos, time: usize },
    Edge { agent: usize, from: Pos, to: Pos, time: usize },
}

#[derive(Clone, Debug)]
enum Conflict {
    Vertex { a: usize, b: usize, pos: Pos, time: usize },
    Edge { a: usize, b: usize, a_from: Pos, a_to: Pos, b_from: Pos, b_to: Pos, time: usize },
}

// ── A* ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
struct AStarItem {
    f: usize,
    g: usize,
    pos: Pos,
    time: usize,
}

impl Ord for AStarItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.cmp(&self.f)
            .then_with(|| other.g.cmp(&self.g))
            .then_with(|| other.time.cmp(&self.time))
            .then_with(|| other.pos.cmp(&self.pos))
    }
}
impl PartialOrd for AStarItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

fn violates_vertex(agent: usize, pos: Pos, time: usize, cs: &[Constraint]) -> bool {
    cs.iter().any(|c| matches!(c, Constraint::Vertex { agent: a, pos: p, time: t }
        if *a == agent && *p == pos && *t == time))
}

fn violates_edge(agent: usize, from: Pos, to: Pos, time: usize, cs: &[Constraint]) -> bool {
    cs.iter().any(|c| matches!(c, Constraint::Edge { agent: a, from: f, to: tt, time: t }
        if *a == agent && *f == from && *tt == to && *t == time))
}

fn latest_constraint_time(agent: usize, cs: &[Constraint]) -> usize {
    cs.iter().filter_map(|c| match c {
        Constraint::Vertex { agent: a, time, .. } if *a == agent => Some(*time),
        Constraint::Edge   { agent: a, time, .. } if *a == agent => Some(*time + 1),
        _ => None,
    }).max().unwrap_or(0)
}

fn safe_to_finish(agent: usize, goal: Pos, time: usize, cs: &[Constraint]) -> bool {
    cs.iter().all(|c| !matches!(c, Constraint::Vertex { agent: a, pos, time: t }
        if *a == agent && *pos == goal && *t >= time))
}

fn reconstruct(mut cur: (Pos, usize), came: &HashMap<(Pos, usize), (Pos, usize)>) -> Vec<Pos> {
    let mut path = vec![cur.0];
    while let Some(prev) = came.get(&cur) { cur = *prev; path.push(cur.0); }
    path.reverse();
    path
}

fn a_star(grid: &Grid, start: Pos, goal: Pos, agent: usize, cs: &[Constraint]) -> Option<Vec<Pos>> {
    if violates_vertex(agent, start, 0, cs) { return None; }
    let latest = latest_constraint_time(agent, cs);
    let max_time = latest + (grid.width * grid.height) as usize * 4 + 20;

    let mut open = BinaryHeap::new();
    let mut visited = HashSet::<(Pos, usize)>::new();
    let mut came: HashMap<(Pos, usize), (Pos, usize)> = HashMap::new();

    open.push(AStarItem { f: start.manhattan(goal), g: 0, pos: start, time: 0 });

    while let Some(cur) = open.pop() {
        let key = (cur.pos, cur.time);
        if !visited.insert(key) { continue; }
        if cur.pos == goal && safe_to_finish(agent, goal, cur.time, cs) {
            return Some(reconstruct(key, &came));
        }
        if cur.time >= max_time { continue; }
        for next in grid.neighbors_with_wait(cur.pos) {
            let nt = cur.time + 1;
            if violates_vertex(agent, next, nt, cs) { continue; }
            if violates_edge(agent, cur.pos, next, cur.time, cs) { continue; }
            let nk = (next, nt);
            if visited.contains(&nk) || came.contains_key(&nk) { continue; }
            came.insert(nk, key);
            let g = nt;
            open.push(AStarItem { f: g + next.manhattan(goal), g, pos: next, time: nt });
        }
    }
    None
}

// ── CBS ─────────────────────────────────────────────────────────────────────

fn position_at(path: &[Pos], time: usize) -> Pos {
    if time < path.len() { path[time] } else { *path.last().unwrap() }
}

fn find_first_conflict(paths: &[Vec<Pos>]) -> Option<Conflict> {
    let horizon = paths.iter().map(|p| p.len()).max().unwrap_or(0);
    for time in 0..horizon {
        for a in 0..paths.len() {
            for b in (a + 1)..paths.len() {
                let ap = position_at(&paths[a], time);
                let bp = position_at(&paths[b], time);
                if ap == bp { return Some(Conflict::Vertex { a, b, pos: ap, time }); }
                if time + 1 < horizon {
                    let an = position_at(&paths[a], time + 1);
                    let bn = position_at(&paths[b], time + 1);
                    if ap == bn && an == bp && ap != an {
                        return Some(Conflict::Edge { a, b, a_from: ap, a_to: an, b_from: bp, b_to: bn, time });
                    }
                }
            }
        }
    }
    None
}

fn count_conflicts(paths: &[Vec<Pos>]) -> usize {
    let horizon = paths.iter().map(|p| p.len()).max().unwrap_or(0);
    let mut n = 0;
    for time in 0..horizon {
        for a in 0..paths.len() {
            for b in (a + 1)..paths.len() {
                let ap = position_at(&paths[a], time);
                let bp = position_at(&paths[b], time);
                if ap == bp { n += 1; }
                if time + 1 < horizon {
                    let an = position_at(&paths[a], time + 1);
                    let bn = position_at(&paths[b], time + 1);
                    if ap == bn && an == bp && ap != an { n += 1; }
                }
            }
        }
    }
    n
}

fn cost_of(paths: &[Vec<Pos>]) -> usize {
    paths.iter().map(|p| p.len().saturating_sub(1)).sum()
}

fn constraint_from(conflict: &Conflict, agent: usize) -> Constraint {
    match conflict {
        Conflict::Vertex { pos, time, .. } => Constraint::Vertex { agent, pos: *pos, time: *time },
        Conflict::Edge { a, a_from, a_to, b_from, b_to, time, .. } => {
            if agent == *a {
                Constraint::Edge { agent, from: *a_from, to: *a_to, time: *time }
            } else {
                Constraint::Edge { agent, from: *b_from, to: *b_to, time: *time }
            }
        }
    }
}

fn conflict_agents(c: &Conflict) -> [usize; 2] {
    match c {
        Conflict::Vertex { a, b, .. } | Conflict::Edge { a, b, .. } => [*a, *b],
    }
}

#[derive(Clone, Debug)]
struct CbsNode {
    constraints: Vec<Constraint>,
    paths: Vec<Vec<Pos>>,
    cost: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OpenItem {
    cost: usize,
    conflicts: usize,
    id: usize,
    node_index: usize,
}
impl Ord for OpenItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.cmp(&self.cost)
            .then_with(|| other.conflicts.cmp(&self.conflicts))
            .then_with(|| other.id.cmp(&self.id))
            .then_with(|| other.node_index.cmp(&self.node_index))
    }
}
impl PartialOrd for OpenItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

fn cbs(grid: &Grid, agents: &[Agent]) -> Option<Vec<Vec<Pos>>> {
    let mut root_paths = Vec::new();
    for (i, agent) in agents.iter().enumerate() {
        root_paths.push(a_star(grid, agent.start, agent.goal, i, &[])?);
    }

    let root = CbsNode { cost: cost_of(&root_paths), constraints: vec![], paths: root_paths };
    let mut nodes = vec![root];
    let mut open = BinaryHeap::new();
    let mut next_id = 1usize;

    open.push(OpenItem { cost: nodes[0].cost, conflicts: count_conflicts(&nodes[0].paths), id: 0, node_index: 0 });

    while let Some(item) = open.pop() {
        let node = nodes[item.node_index].clone();
        let Some(conflict) = find_first_conflict(&node.paths) else {
            return Some(node.paths);
        };
        for ai in conflict_agents(&conflict) {
            let nc = constraint_from(&conflict, ai);
            if node.constraints.contains(&nc) { continue; }
            let mut next_cs = node.constraints.clone();
            next_cs.push(nc);
            let mut next_paths = node.paths.clone();
            if let Some(p) = a_star(grid, agents[ai].start, agents[ai].goal, ai, &next_cs) {
                next_paths[ai] = p;
                let cost = cost_of(&next_paths);
                let conflicts = count_conflicts(&next_paths);
                let ni = nodes.len();
                nodes.push(CbsNode { constraints: next_cs, paths: next_paths, cost });
                open.push(OpenItem { cost, conflicts, id: next_id, node_index: ni });
                next_id += 1;
            }
        }
        if nodes.len() > 50_000 { break; }
    }
    None
}

// ── app ─────────────────────────────────────────────────────────────────────

const AGENT_COLORS: [Color32; 5] = [
    Color32::from_rgb(0x4a, 0x9e, 0xff),
    Color32::from_rgb(0xff, 0x7b, 0x54),
    Color32::from_rgb(0x5d, 0xd8, 0x8a),
    Color32::from_rgb(0xff, 0xd1, 0x66),
    Color32::from_rgb(0xc0, 0x8a, 0xff),
];

struct CbsApp {
    grid: Grid,
    agents: Vec<Agent>,
    paths: Option<Vec<Vec<Pos>>>,
    error: Option<String>,

    time: usize,
    horizon: usize,
    playing: bool,
    play_accum: f64,
    play_speed: f64,

    cell_px: f32,
}

impl Default for CbsApp {
    fn default() -> Self {
        let grid = Grid::new(
            8,
            6,
            &[(3, 0), (3, 1), (3, 3), (3, 4), (3, 5)],
        );
        let agents = vec![
            Agent { name: "A".into(), start: Pos::new(0, 2), goal: Pos::new(7, 2), color: AGENT_COLORS[0] },
            Agent { name: "B".into(), start: Pos::new(7, 2), goal: Pos::new(0, 2), color: AGENT_COLORS[1] },
            Agent { name: "C".into(), start: Pos::new(1, 0), goal: Pos::new(1, 5), color: AGENT_COLORS[2] },
        ];
        let (paths, error) = match cbs(&grid, &agents) {
            Some(p) => (Some(p), None),
            None => (None, Some("CBS: no solution found".into())),
        };
        let horizon = paths.as_ref().map(|p| p.iter().map(|r| r.len()).max().unwrap_or(1)).unwrap_or(1);
        Self { grid, agents, paths, error, time: 0, horizon, playing: false, play_accum: 0.0, play_speed: 1.5, cell_px: 72.0 }
    }
}

impl eframe::App for CbsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.playing {
            let dt = ctx.input(|i| i.unstable_dt) as f64;
            self.play_accum += dt * self.play_speed;
            let steps = self.play_accum as usize;
            if steps > 0 {
                self.play_accum -= steps as f64;
                self.time = (self.time + steps).min(self.horizon.saturating_sub(1));
                if self.time + 1 >= self.horizon {
                    self.playing = false;
                }
            }
            ctx.request_repaint();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(0x14, 0x17, 0x1c)))
            .show(ctx, |ui| {
                ui.add_space(16.0);

                // ── title ──
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new("CBS Multi-Agent Pathfinding")
                            .color(Color32::from_rgb(0xee, 0xf2, 0xf8))
                            .size(22.0)
                            .strong(),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new("Conflict-Based Search が複数エージェントの衝突なし経路を探索します")
                            .color(Color32::from_rgb(0xae, 0xb8, 0xc7))
                            .size(14.0),
                    );
                });
                ui.add_space(16.0);

                // ── main layout: grid + sidebar ──
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    self.draw_grid(ui);
                    ui.add_space(32.0);
                    self.draw_sidebar(ui);
                });

                ui.add_space(20.0);

                // ── timeline controls ──
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    self.draw_controls(ui);
                });
            });
    }
}

impl CbsApp {
    fn draw_grid(&mut self, ui: &mut egui::Ui) {
        let w = self.grid.width as f32;
        let h = self.grid.height as f32;
        let px = self.cell_px;
        let total = Vec2::new(w * px, h * px);

        let (resp, painter) = ui.allocate_painter(total, Sense::hover());
        let origin = resp.rect.min;

        let cell_rect = |x: i32, y: i32| {
            let tl = origin + Vec2::new(x as f32 * px, y as f32 * px);
            Rect::from_min_size(tl, Vec2::splat(px))
        };

        // background cells
        for y in 0..self.grid.height {
            for x in 0..self.grid.width {
                let p = Pos::new(x, y);
                let r = cell_rect(x, y);
                let fill = if self.grid.walls.contains(&p) {
                    Color32::from_rgb(0x2a, 0x2f, 0x3a)
                } else {
                    Color32::from_rgb(0x1d, 0x22, 0x2b)
                };
                painter.rect_filled(r, 2.0, fill);
                painter.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_rgb(0x30, 0x38, 0x46)), StrokeKind::Outside);
            }
        }

        // wall hatching
        for y in 0..self.grid.height {
            for x in 0..self.grid.width {
                let p = Pos::new(x, y);
                if self.grid.walls.contains(&p) {
                    let r = cell_rect(x, y);
                    painter.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        "▪",
                        FontId::proportional(px * 0.55),
                        Color32::from_rgb(0x3c, 0x44, 0x54),
                    );
                }
            }
        }

        if let Some(paths) = &self.paths {
            // path trails
            for (i, path) in paths.iter().enumerate() {
                let color = self.agents[i].color.gamma_multiply(0.28);
                for win in path.windows(2) {
                    let a = win[0];
                    let b = win[1];
                    let pa = cell_rect(a.x, a.y).center();
                    let pb = cell_rect(b.x, b.y).center();
                    painter.line_segment([pa, pb], Stroke::new(3.0, color));
                }
                // dots on each step
                for pos in path.iter() {
                    let c = cell_rect(pos.x, pos.y).center();
                    painter.circle_filled(c, 3.5, self.agents[i].color.gamma_multiply(0.45));
                }
            }

            // start / goal markers
            for (i, agent) in self.agents.iter().enumerate() {
                let sc = cell_rect(agent.start.x, agent.start.y).center();
                let gc = cell_rect(agent.goal.x, agent.goal.y).center();
                let col = agent.color;
                // start: hollow ring
                painter.circle_stroke(sc, px * 0.32, Stroke::new(2.0, col.gamma_multiply(0.6)));
                // goal: filled diamond via text ◆
                painter.text(gc, Align2::CENTER_CENTER, "◆",
                    FontId::proportional(px * 0.38), col.gamma_multiply(0.55));
                // goal label
                let label = format!("{}", &paths[i].len().saturating_sub(1));
                painter.text(
                    gc + Vec2::new(px * 0.3, -px * 0.3),
                    Align2::LEFT_BOTTOM,
                    label,
                    FontId::proportional(9.0),
                    col.gamma_multiply(0.6),
                );
            }

            // agents at current time
            for (i, path) in paths.iter().enumerate() {
                let pos = position_at(path, self.time);
                let center = cell_rect(pos.x, pos.y).center();
                let col = self.agents[i].color;
                painter.circle_filled(center, px * 0.34, col);
                painter.circle_stroke(center, px * 0.34, Stroke::new(2.0, Color32::WHITE.gamma_multiply(0.3)));
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    &self.agents[i].name,
                    FontId::proportional(px * 0.36).clone(),
                    Color32::from_rgb(0x10, 0x13, 0x18),
                );
            }
        } else {
            let center = origin + total / 2.0;
            painter.text(
                center,
                Align2::CENTER_CENTER,
                self.error.as_deref().unwrap_or("No solution"),
                FontId::proportional(16.0),
                Color32::from_rgb(0xff, 0x7b, 0x54),
            );
        }
    }

    fn draw_sidebar(&self, ui: &mut egui::Ui) {
        let accent = Color32::from_rgb(0x8f, 0xba, 0xff);
        let muted = Color32::from_rgb(0xae, 0xb8, 0xc7);
        let dim = Color32::from_rgb(0x60, 0x6c, 0x80);

        ui.vertical(|ui| {
            ui.set_min_width(220.0);

            ui.label(egui::RichText::new("エージェント").color(accent).size(13.0).strong());
            ui.add_space(6.0);

            for agent in &self.agents {
                ui.horizontal(|ui| {
                    // color swatch
                    let (r, p) = ui.allocate_painter(Vec2::splat(14.0), Sense::hover());
                    p.circle_filled(r.rect.center(), 6.0, agent.color);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: ({},{}) → ({},{})",
                            agent.name, agent.start.x, agent.start.y, agent.goal.x, agent.goal.y
                        ))
                        .color(muted)
                        .size(13.0),
                    );
                });
            }

            ui.add_space(20.0);
            ui.label(egui::RichText::new("凡例").color(accent).size(13.0).strong());
            ui.add_space(6.0);
            for (sym, desc) in [
                ("○", "スタート"),
                ("◆", "ゴール"),
                ("●", "現在位置"),
                ("▪", "壁"),
            ] {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(sym).color(muted).size(15.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(desc).color(dim).size(13.0));
                });
            }

            if let Some(paths) = &self.paths {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("経路詳細").color(accent).size(13.0).strong());
                ui.add_space(6.0);
                for (i, path) in paths.iter().enumerate() {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {} ステップ",
                            self.agents[i].name,
                            path.len().saturating_sub(1)
                        ))
                        .color(self.agents[i].color)
                        .size(13.0),
                    );
                    let pos = position_at(path, self.time);
                    ui.label(
                        egui::RichText::new(format!(
                            "  t={} → ({}, {})",
                            self.time, pos.x, pos.y
                        ))
                        .color(dim)
                        .size(12.0),
                    );
                }

                ui.add_space(20.0);
                ui.label(egui::RichText::new("アルゴリズム").color(accent).size(13.0).strong());
                ui.add_space(6.0);
                let total_cost: usize = paths.iter().map(|p| p.len().saturating_sub(1)).sum();
                ui.label(egui::RichText::new(format!("総コスト: {total_cost}")).color(muted).size(13.0));
                ui.label(egui::RichText::new("CBS (high-level)").color(dim).size(12.0));
                ui.label(egui::RichText::new("制約付きA* (low-level)").color(dim).size(12.0));
            }
        });
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        let muted = Color32::from_rgb(0xae, 0xb8, 0xc7);
        let dim = Color32::from_rgb(0x60, 0x6c, 0x80);

        // time = N / horizon display
        ui.label(
            egui::RichText::new(format!("t = {} / {}", self.time, self.horizon.saturating_sub(1)))
                .color(muted)
                .size(14.0),
        );
        ui.add_space(8.0);

        // slider
        let mut t = self.time as f32;
        let max = (self.horizon.saturating_sub(1)) as f32;
        if ui.add(egui::Slider::new(&mut t, 0.0..=max).show_value(false)).changed() {
            self.time = t as usize;
            self.playing = false;
        }
        ui.add_space(8.0);

        // buttons
        ui.horizontal(|ui| {
            if ui.button("|◀").clicked() {
                self.time = 0;
                self.playing = false;
            }
            if ui.button("◀").clicked() && self.time > 0 {
                self.time -= 1;
                self.playing = false;
            }
            let play_label = if self.playing { "⏸" } else { "▶" };
            if ui.button(play_label).clicked() {
                if self.time + 1 >= self.horizon {
                    self.time = 0;
                }
                self.playing = !self.playing;
            }
            if ui.button("▶").clicked() && self.time + 1 < self.horizon {
                self.time += 1;
                self.playing = false;
            }
            if ui.button("▶|").clicked() {
                self.time = self.horizon.saturating_sub(1);
                self.playing = false;
            }
            ui.add_space(16.0);
            ui.label(egui::RichText::new("速度:").color(dim).size(13.0));
            ui.add(egui::Slider::new(&mut self.play_speed, 0.25..=4.0).text("").fixed_decimals(2));
        });
    }
}

// ── entry points ─────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "CBS Multi-Agent Pathfinding Visualizer",
        native_options,
        Box::new(|_cc| Ok(Box::new(CbsApp::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::{wasm_bindgen::JsCast as _, web_sys};
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("cbs-visualizer"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing #cbs-visualizer canvas");
        eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|_cc| Ok(Box::new(CbsApp::default()))))
            .await
            .expect("failed to start eframe web app");
    });
}
