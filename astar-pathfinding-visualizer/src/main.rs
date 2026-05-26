use eframe::egui;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Pos {
    x: usize,
    y: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Empty,
    Wall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditMode {
    Wall,
    Start,
    Goal,
}

#[derive(Clone, Debug)]
struct Node {
    pos: Pos,
    f_score: usize,
    g_score: usize,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap は最大ヒープなので、スコアが小さいものを優先するため逆順にする
        other
            .f_score
            .cmp(&self.f_score)
            .then_with(|| other.g_score.cmp(&self.g_score))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.pos == other.pos
    }
}

impl Eq for Node {}

struct AStarState {
    open_set: BinaryHeap<Node>,
    open_positions: HashSet<Pos>,
    closed_set: HashSet<Pos>,
    came_from: HashMap<Pos, Pos>,
    g_score: HashMap<Pos, usize>,
    path: Vec<Pos>,
    finished: bool,
    found: bool,
}

impl AStarState {
    fn new(start: Pos, goal: Pos) -> Self {
        let mut open_set = BinaryHeap::new();
        let mut open_positions = HashSet::new();
        let mut g_score = HashMap::new();

        let h = heuristic(start, goal);

        open_set.push(Node {
            pos: start,
            f_score: h,
            g_score: 0,
        });

        open_positions.insert(start);
        g_score.insert(start, 0);

        Self {
            open_set,
            open_positions,
            closed_set: HashSet::new(),
            came_from: HashMap::new(),
            g_score,
            path: Vec::new(),
            finished: false,
            found: false,
        }
    }
}

struct AStarApp {
    width: usize,
    height: usize,
    grid: Vec<Cell>,

    start: Pos,
    goal: Pos,

    edit_mode: EditMode,

    astar: Option<AStarState>,
    running: bool,

    steps_per_frame: usize,
}

impl Default for AStarApp {
    fn default() -> Self {
        let width = 40;
        let height = 28;

        Self {
            width,
            height,
            grid: vec![Cell::Empty; width * height],

            start: Pos { x: 3, y: 3 },
            goal: Pos { x: 35, y: 22 },

            edit_mode: EditMode::Wall,

            astar: None,
            running: false,

            steps_per_frame: 3,
        }
    }
}

impl AStarApp {
    fn index(&self, pos: Pos) -> usize {
        pos.y * self.width + pos.x
    }

    fn cell(&self, pos: Pos) -> Cell {
        self.grid[self.index(pos)]
    }

    fn set_cell(&mut self, pos: Pos, cell: Cell) {
        let index = self.index(pos);
        self.grid[index] = cell;
    }

    fn in_bounds(&self, pos: Pos) -> bool {
        pos.x < self.width && pos.y < self.height
    }

    fn reset_search(&mut self) {
        self.astar = None;
        self.running = false;
    }

    fn start_search(&mut self) {
        self.astar = Some(AStarState::new(self.start, self.goal));
        self.running = true;
    }

    fn clear_walls(&mut self) {
        for cell in &mut self.grid {
            *cell = Cell::Empty;
        }
        self.reset_search();
    }

    fn generate_maze_like_walls(&mut self) {
        self.clear_walls();

        for y in 0..self.height {
            for x in 0..self.width {
                let pos = Pos { x, y };

                if pos == self.start || pos == self.goal {
                    continue;
                }

                let wall = (x % 7 == 0 && y % 3 != 0) || (y % 6 == 0 && x % 5 != 0);

                if wall {
                    self.set_cell(pos, Cell::Wall);
                }
            }
        }

        self.reset_search();
    }

    fn step_astar(&mut self) {
        // Take state out so we can borrow self.grid immutably while mutating state.
        let mut state = match self.astar.take() {
            Some(s) => s,
            None => return,
        };

        if state.finished {
            self.astar = Some(state);
            return;
        }

        let Some(current_node) = state.open_set.pop() else {
            state.finished = true;
            state.found = false;
            self.astar = Some(state);
            return;
        };

        let current = current_node.pos;
        state.open_positions.remove(&current);

        if current == self.goal {
            state.finished = true;
            state.found = true;
            state.path = reconstruct_path(&state.came_from, current);
            self.astar = Some(state);
            return;
        }

        state.closed_set.insert(current);

        for neighbor in neighbors(current, self.width, self.height) {
            if self.cell(neighbor) == Cell::Wall {
                continue;
            }

            if state.closed_set.contains(&neighbor) {
                continue;
            }

            let tentative_g = state
                .g_score
                .get(&current)
                .copied()
                .unwrap_or(usize::MAX)
                + 1;

            let known_g = state
                .g_score
                .get(&neighbor)
                .copied()
                .unwrap_or(usize::MAX);

            if tentative_g < known_g {
                state.came_from.insert(neighbor, current);
                state.g_score.insert(neighbor, tentative_g);

                let f_score = tentative_g + heuristic(neighbor, self.goal);

                state.open_set.push(Node {
                    pos: neighbor,
                    f_score,
                    g_score: tentative_g,
                });

                state.open_positions.insert(neighbor);
            }
        }

        self.astar = Some(state);
    }

    fn handle_cell_click(&mut self, pos: Pos) {
        match self.edit_mode {
            EditMode::Wall => {
                if pos != self.start && pos != self.goal {
                    let next = match self.cell(pos) {
                        Cell::Empty => Cell::Wall,
                        Cell::Wall => Cell::Empty,
                    };
                    self.set_cell(pos, next);
                    self.reset_search();
                }
            }
            EditMode::Start => {
                if pos != self.goal && self.cell(pos) != Cell::Wall {
                    self.start = pos;
                    self.reset_search();
                }
            }
            EditMode::Goal => {
                if pos != self.start && self.cell(pos) != Cell::Wall {
                    self.goal = pos;
                    self.reset_search();
                }
            }
        }
    }
}

impl eframe::App for AStarApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.running {
            for _ in 0..self.steps_per_frame {
                self.step_astar();
            }

            if let Some(state) = &self.astar {
                if state.finished {
                    self.running = false;
                }
            }

            ctx.request_repaint();
        }

        egui::SidePanel::left("control_panel")
            .resizable(false)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.heading("A* Visualizer");

                ui.separator();

                ui.label("Edit mode");

                ui.radio_value(&mut self.edit_mode, EditMode::Wall, "Wall");
                ui.radio_value(&mut self.edit_mode, EditMode::Start, "Start");
                ui.radio_value(&mut self.edit_mode, EditMode::Goal, "Goal");

                ui.separator();

                if ui.button("Start").clicked() {
                    self.start_search();
                }

                if ui.button("Step").clicked() {
                    if self.astar.is_none() {
                        self.astar = Some(AStarState::new(self.start, self.goal));
                    }
                    self.step_astar();
                }

                if ui.button("Pause").clicked() {
                    self.running = false;
                }

                if ui.button("Reset Search").clicked() {
                    self.reset_search();
                }

                if ui.button("Clear Walls").clicked() {
                    self.clear_walls();
                }

                if ui.button("Generate Walls").clicked() {
                    self.generate_maze_like_walls();
                }

                ui.separator();

                ui.add(
                    egui::Slider::new(&mut self.steps_per_frame, 1..=50)
                        .text("steps/frame"),
                );

                ui.separator();

                if let Some(state) = &self.astar {
                    ui.label(format!("open: {}", state.open_positions.len()));
                    ui.label(format!("closed: {}", state.closed_set.len()));

                    if state.finished && state.found {
                        ui.label(format!("path length: {}", state.path.len()));
                    } else if state.finished {
                        ui.label("path not found");
                    }
                }

                ui.separator();

                ui.label("Color:");
                ui.label("Green = Start");
                ui.label("Red = Goal");
                ui.label("Black = Wall");
                ui.label("Blue = Open");
                ui.label("Gray = Closed");
                ui.label("Yellow = Path");
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();

            let cell_size_x = available.x / self.width as f32;
            let cell_size_y = available.y / self.height as f32;
            let cell_size = cell_size_x.min(cell_size_y).floor();

            let grid_width = cell_size * self.width as f32;
            let grid_height = cell_size * self.height as f32;

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(grid_width, grid_height),
                egui::Sense::click_and_drag(),
            );

            let painter = ui.painter_at(rect);

            if let Some(pointer_pos) = response.interact_pointer_pos() {
                if response.clicked() || response.dragged() {
                    let local_x = pointer_pos.x - rect.left();
                    let local_y = pointer_pos.y - rect.top();

                    let x = (local_x / cell_size) as usize;
                    let y = (local_y / cell_size) as usize;

                    let pos = Pos { x, y };

                    if self.in_bounds(pos) {
                        self.handle_cell_click(pos);
                    }
                }
            }

            let astar = self.astar.as_ref();

            for y in 0..self.height {
                for x in 0..self.width {
                    let pos = Pos { x, y };

                    let x0 = rect.left() + x as f32 * cell_size;
                    let y0 = rect.top() + y as f32 * cell_size;

                    let cell_rect = egui::Rect::from_min_size(
                        egui::pos2(x0, y0),
                        egui::vec2(cell_size - 1.0, cell_size - 1.0),
                    );

                    let color = cell_color(self, astar, pos);

                    painter.rect_filled(cell_rect, 0.0, color);
                }
            }
        });
    }
}

fn cell_color(app: &AStarApp, astar: Option<&AStarState>, pos: Pos) -> egui::Color32 {
    if pos == app.start {
        return egui::Color32::from_rgb(80, 220, 120);
    }

    if pos == app.goal {
        return egui::Color32::from_rgb(240, 80, 80);
    }

    if let Some(state) = astar {
        if state.path.contains(&pos) {
            return egui::Color32::from_rgb(250, 220, 80);
        }

        if state.open_positions.contains(&pos) {
            return egui::Color32::from_rgb(80, 150, 250);
        }

        if state.closed_set.contains(&pos) {
            return egui::Color32::from_rgb(120, 120, 120);
        }
    }

    match app.cell(pos) {
        Cell::Empty => egui::Color32::from_rgb(235, 235, 235),
        Cell::Wall => egui::Color32::from_rgb(30, 30, 30),
    }
}

fn neighbors(pos: Pos, width: usize, height: usize) -> Vec<Pos> {
    let mut result = Vec::new();

    if pos.x > 0 {
        result.push(Pos {
            x: pos.x - 1,
            y: pos.y,
        });
    }

    if pos.x + 1 < width {
        result.push(Pos {
            x: pos.x + 1,
            y: pos.y,
        });
    }

    if pos.y > 0 {
        result.push(Pos {
            x: pos.x,
            y: pos.y - 1,
        });
    }

    if pos.y + 1 < height {
        result.push(Pos {
            x: pos.x,
            y: pos.y + 1,
        });
    }

    result
}

fn heuristic(a: Pos, b: Pos) -> usize {
    a.x.abs_diff(b.x) + a.y.abs_diff(b.y)
}

fn reconstruct_path(came_from: &HashMap<Pos, Pos>, mut current: Pos) -> Vec<Pos> {
    let mut path = vec![current];

    while let Some(prev) = came_from.get(&current) {
        current = *prev;
        path.push(current);
    }

    path.reverse();
    path
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "A* Pathfinding Visualizer",
        options,
        Box::new(|_cc| Ok(Box::new(AStarApp::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::{wasm_bindgen::JsCast as _, web_sys};
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("astar-pathfinding-visualizer"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing #astar-pathfinding-visualizer canvas");
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(AStarApp::default()))),
            )
            .await
            .expect("failed to start eframe web app");
    });
}
