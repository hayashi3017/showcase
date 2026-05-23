use std::collections::VecDeque;

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2,
};

const N: usize = 0;
const E: usize = 1;
const S: usize = 2;
const W: usize = 3;

const DX: [isize; 4] = [0, 1, 0, -1];
const DY: [isize; 4] = [-1, 0, 1, 0];
const OPPOSITE: [usize; 4] = [S, W, N, E];

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "WFC Road Visualizer",
        native_options,
        Box::new(|_cc| Ok(Box::new(WfcApp::default()))),
    )
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    mask: u8,
    weight: u32,
}

#[derive(Debug)]
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x1234_5678_9abc_def0
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn gen_range(&mut self, upper: usize) -> usize {
        assert!(upper > 0);
        (self.next_u64() as usize) % upper
    }

    fn choose_weighted_tile(&mut self, possible: u64, tiles: &[Tile]) -> usize {
        let total_weight: u32 = tiles
            .iter()
            .enumerate()
            .filter(|(i, _)| has_tile(possible, *i))
            .map(|(_, tile)| tile.weight)
            .sum();

        let mut r = (self.next_u64() % total_weight as u64) as u32;

        for (i, tile) in tiles.iter().enumerate() {
            if !has_tile(possible, i) {
                continue;
            }

            if r < tile.weight {
                return i;
            }

            r -= tile.weight;
        }

        unreachable!()
    }
}

#[derive(Debug)]
struct Wfc {
    width: usize,
    height: usize,
    tiles: Vec<Tile>,
    compatible: Vec<[u64; 4]>,
    cells: Vec<u64>,
    #[allow(dead_code)]
    all_tiles_mask: u64,
}

#[derive(Debug)]
struct StepReport {
    collapsed: Option<usize>,
    changed: Vec<usize>,
    message: String,
    finished: bool,
}

impl Wfc {
    fn new(width: usize, height: usize) -> Self {
        let tiles = road_tiles();
        assert!(tiles.len() <= 64);

        let all_tiles_mask = (1u64 << tiles.len()) - 1;
        let mut compatible = vec![[0u64; 4]; tiles.len()];

        for (a_idx, a) in tiles.iter().enumerate() {
            for dir in 0..4 {
                let mut allowed = 0u64;

                for (b_idx, b) in tiles.iter().enumerate() {
                    let a_edge = edge(a.mask, dir);
                    let b_edge = edge(b.mask, OPPOSITE[dir]);

                    if a_edge == b_edge {
                        allowed |= 1u64 << b_idx;
                    }
                }

                compatible[a_idx][dir] = allowed;
            }
        }

        Self {
            width,
            height,
            tiles,
            compatible,
            cells: vec![all_tiles_mask; width * height],
            all_tiles_mask,
        }
    }

    #[allow(dead_code)]
    fn reset(&mut self) {
        self.cells.fill(self.all_tiles_mask);
    }

    fn apply_closed_boundary(&mut self) -> Result<Vec<usize>, String> {
        let mut queue = VecDeque::new();
        let mut changed = Vec::new();

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = self.index(x, y);
                let mut allowed = self.cells[idx];

                if y == 0 {
                    allowed &= self.tiles_with_edge(N, 0);
                }
                if x + 1 == self.width {
                    allowed &= self.tiles_with_edge(E, 0);
                }
                if y + 1 == self.height {
                    allowed &= self.tiles_with_edge(S, 0);
                }
                if x == 0 {
                    allowed &= self.tiles_with_edge(W, 0);
                }

                if allowed == 0 {
                    return Err(format!("boundary contradiction at ({x}, {y})"));
                }

                if allowed != self.cells[idx] {
                    self.cells[idx] = allowed;
                    changed.push(idx);
                    queue.push_back((x, y));
                }
            }
        }

        let mut propagated = self.propagate(queue)?;
        changed.append(&mut propagated);

        Ok(changed)
    }

    fn seed_some_roads(
        &mut self,
        rng: &mut Rng,
        count: usize,
    ) -> Result<Vec<usize>, String> {
        if self.width < 3 || self.height < 3 || count == 0 {
            return Ok(Vec::new());
        }

        let road_masks = [
            5u8,  // vertical
            10u8, // horizontal
            3u8,  // corner
            6u8,
            9u8,
            12u8,
            7u8,  // T
            11u8,
            13u8,
            14u8,
            15u8, // cross
        ];

        let mut all_changed = Vec::new();

        for _ in 0..count {
            for _attempt in 0..40 {
                let x = 1 + rng.gen_range(self.width - 2);
                let y = 1 + rng.gen_range(self.height - 2);
                let idx = self.index(x, y);

                let tile_index = road_masks[rng.gen_range(road_masks.len())] as usize;
                let chosen = 1u64 << tile_index;

                if self.cells[idx] & chosen == 0 {
                    continue;
                }

                self.cells[idx] = chosen;

                let mut queue = VecDeque::new();
                queue.push_back((x, y));

                all_changed.push(idx);
                let mut propagated = self.propagate(queue)?;
                all_changed.append(&mut propagated);

                break;
            }
        }

        Ok(all_changed)
    }

    fn step(&mut self, rng: &mut Rng) -> Result<StepReport, String> {
        let Some(cell_index) = self.pick_lowest_entropy_cell(rng) else {
            return Ok(StepReport {
                collapsed: None,
                changed: Vec::new(),
                message: "finished".to_string(),
                finished: true,
            });
        };

        let possible = self.cells[cell_index];

        if possible == 0 {
            return Err(format!("cell {cell_index} has no candidates"));
        }

        let chosen_tile = rng.choose_weighted_tile(possible, &self.tiles);
        self.cells[cell_index] = 1u64 << chosen_tile;

        let x = cell_index % self.width;
        let y = cell_index / self.width;

        let mut queue = VecDeque::new();
        queue.push_back((x, y));

        let changed = self.propagate(queue)?;

        Ok(StepReport {
            collapsed: Some(cell_index),
            changed,
            message: format!(
                "collapsed ({x}, {y}) to mask {:04b}",
                self.tiles[chosen_tile].mask
            ),
            finished: false,
        })
    }

    fn propagate(
        &mut self,
        mut queue: VecDeque<(usize, usize)>,
    ) -> Result<Vec<usize>, String> {
        let mut changed = Vec::new();

        while let Some((x, y)) = queue.pop_front() {
            let idx = self.index(x, y);
            let possible_here = self.cells[idx];

            for dir in 0..4 {
                let Some((nx, ny)) = self.neighbor(x, y, dir) else {
                    continue;
                };

                let neighbor_idx = self.index(nx, ny);
                let allowed_for_neighbor =
                    self.allowed_neighbors(possible_here, dir);

                let next_possible =
                    self.cells[neighbor_idx] & allowed_for_neighbor;

                if next_possible == 0 {
                    return Err(format!(
                        "contradiction: ({nx}, {ny}) has no possible tiles"
                    ));
                }

                if next_possible != self.cells[neighbor_idx] {
                    self.cells[neighbor_idx] = next_possible;
                    changed.push(neighbor_idx);
                    queue.push_back((nx, ny));
                }
            }
        }

        Ok(changed)
    }

    fn pick_lowest_entropy_cell(&self, rng: &mut Rng) -> Option<usize> {
        let mut best_count = u32::MAX;
        let mut candidates = Vec::new();

        for (i, &possible) in self.cells.iter().enumerate() {
            let count = possible.count_ones();

            if count <= 1 {
                continue;
            }

            if count < best_count {
                best_count = count;
                candidates.clear();
                candidates.push(i);
            } else if count == best_count {
                candidates.push(i);
            }
        }

        if candidates.is_empty() {
            None
        } else {
            Some(candidates[rng.gen_range(candidates.len())])
        }
    }

    fn allowed_neighbors(&self, possible_here: u64, dir: usize) -> u64 {
        let mut allowed = 0u64;

        for tile_index in 0..self.tiles.len() {
            if has_tile(possible_here, tile_index) {
                allowed |= self.compatible[tile_index][dir];
            }
        }

        allowed
    }

    fn tiles_with_edge(&self, dir: usize, value: u8) -> u64 {
        let mut result = 0u64;

        for (i, tile) in self.tiles.iter().enumerate() {
            if edge(tile.mask, dir) == value {
                result |= 1u64 << i;
            }
        }

        result
    }

    fn neighbor(&self, x: usize, y: usize, dir: usize) -> Option<(usize, usize)> {
        let nx = x as isize + DX[dir];
        let ny = y as isize + DY[dir];

        if nx < 0 || ny < 0 {
            return None;
        }

        let nx = nx as usize;
        let ny = ny as usize;

        if nx >= self.width || ny >= self.height {
            None
        } else {
            Some((nx, ny))
        }
    }

    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    fn is_collapsed(&self, idx: usize) -> bool {
        self.cells[idx].count_ones() == 1
    }

    fn tile_mask_at(&self, idx: usize) -> Option<u8> {
        if !self.is_collapsed(idx) {
            return None;
        }

        let tile_index = self.cells[idx].trailing_zeros() as usize;
        Some(self.tiles[tile_index].mask)
    }
}

fn road_tiles() -> Vec<Tile> {
    (0u8..16)
        .map(|mask| Tile {
            mask,
            weight: road_weight(mask),
        })
        .collect()
}

fn road_weight(mask: u8) -> u32 {
    match mask.count_ones() {
        0 => 8,
        1 => 1,
        2 => {
            if mask == 5 || mask == 10 {
                8
            } else {
                6
            }
        }
        3 => 3,
        4 => 1,
        _ => unreachable!(),
    }
}

fn edge(mask: u8, dir: usize) -> u8 {
    (mask >> dir) & 1
}

fn has_tile(possible: u64, tile_index: usize) -> bool {
    possible & (1u64 << tile_index) != 0
}

#[derive(Debug)]
struct WfcApp {
    wfc: Wfc,
    rng: Rng,

    width: usize,
    height: usize,
    seed: u64,
    initial_road_count: usize,

    auto: bool,
    steps_per_frame: usize,
    cell_size: f32,

    last_collapsed: Option<usize>,
    last_changed: Vec<usize>,
    status: String,
    finished: bool,
    contradiction: bool,
}

impl Default for WfcApp {
    fn default() -> Self {
        let width = 64;
        let height = 40;
        let seed = 0xC0FFEE;
        let initial_road_count = 8;

        let mut app = Self {
            wfc: Wfc::new(width, height),
            rng: Rng::new(seed),

            width,
            height,
            seed,
            initial_road_count,

            auto: false,
            steps_per_frame: 8,
            cell_size: 18.0,

            last_collapsed: None,
            last_changed: Vec::new(),
            status: String::new(),
            finished: false,
            contradiction: false,
        };

        app.reset_world();
        app
    }
}

impl WfcApp {
    fn reset_world(&mut self) {
        self.wfc = Wfc::new(self.width, self.height);
        self.rng = Rng::new(self.seed);
        self.last_collapsed = None;
        self.last_changed.clear();
        self.finished = false;
        self.contradiction = false;

        match self.wfc.apply_closed_boundary() {
            Ok(mut changed) => {
                match self
                    .wfc
                    .seed_some_roads(&mut self.rng, self.initial_road_count)
                {
                    Ok(mut seeded) => {
                        changed.append(&mut seeded);
                        self.last_changed = changed;
                        self.status = "reset".to_string();
                    }
                    Err(err) => {
                        self.status = err;
                        self.contradiction = true;
                    }
                }
            }
            Err(err) => {
                self.status = err;
                self.contradiction = true;
            }
        }
    }

    fn step_once(&mut self) {
        if self.finished || self.contradiction {
            return;
        }

        match self.wfc.step(&mut self.rng) {
            Ok(report) => {
                self.last_collapsed = report.collapsed;
                self.last_changed = report.changed;
                self.status = report.message;
                self.finished = report.finished;
            }
            Err(err) => {
                self.last_collapsed = None;
                self.last_changed.clear();
                self.status = err;
                self.contradiction = true;
                self.auto = false;
            }
        }
    }

    fn run_to_end(&mut self) {
        let limit = self.width * self.height * 2;

        for _ in 0..limit {
            if self.finished || self.contradiction {
                break;
            }

            self.step_once();
        }
    }

    fn draw_grid(&self, ui: &mut egui::Ui) {
        let desired_size = Vec2::new(
            self.wfc.width as f32 * self.cell_size,
            self.wfc.height as f32 * self.cell_size,
        );

        let (response, painter) =
            ui.allocate_painter(desired_size, Sense::hover());

        let origin = response.rect.min;

        for y in 0..self.wfc.height {
            for x in 0..self.wfc.width {
                let idx = self.wfc.index(x, y);

                let min = Pos2::new(
                    origin.x + x as f32 * self.cell_size,
                    origin.y + y as f32 * self.cell_size,
                );

                let rect = Rect::from_min_size(
                    min,
                    Vec2::new(self.cell_size, self.cell_size),
                );

                self.draw_cell(&painter, rect, idx);
            }
        }
    }

    fn draw_cell(&self, painter: &egui::Painter, rect: Rect, idx: usize) {
        let possible = self.wfc.cells[idx];
        let count = possible.count_ones();

        let mut bg = if count == 1 {
            Color32::from_rgb(35, 42, 48)
        } else {
            entropy_color(count, self.wfc.tiles.len() as u32)
        };

        if self.last_changed.contains(&idx) {
            bg = Color32::from_rgb(30, 90, 170);
        }

        if self.last_collapsed == Some(idx) {
            bg = Color32::from_rgb(220, 170, 40);
        }

        painter.rect_filled(rect.shrink(1.0), 2.0, bg);

        if let Some(mask) = self.wfc.tile_mask_at(idx) {
            self.draw_road(painter, rect, mask);
        } else if self.cell_size >= 14.0 {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                count.to_string(),
                FontId::monospace(self.cell_size * 0.35),
                Color32::WHITE,
            );
        }
    }

    fn draw_road(&self, painter: &egui::Painter, rect: Rect, mask: u8) {
        if mask == 0 {
            return;
        }

        let center = rect.center();
        let stroke = Stroke::new(
            (self.cell_size * 0.16).max(2.0),
            Color32::WHITE,
        );

        if edge(mask, N) == 1 {
            painter.line_segment(
                [center, Pos2::new(center.x, rect.top())],
                stroke,
            );
        }

        if edge(mask, E) == 1 {
            painter.line_segment(
                [center, Pos2::new(rect.right(), center.y)],
                stroke,
            );
        }

        if edge(mask, S) == 1 {
            painter.line_segment(
                [center, Pos2::new(center.x, rect.bottom())],
                stroke,
            );
        }

        if edge(mask, W) == 1 {
            painter.line_segment(
                [center, Pos2::new(rect.left(), center.y)],
                stroke,
            );
        }
    }
}

impl eframe::App for WfcApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.auto && !self.finished && !self.contradiction {
            for _ in 0..self.steps_per_frame {
                self.step_once();

                if self.finished || self.contradiction {
                    break;
                }
            }

            ctx.request_repaint();
        }

        egui::SidePanel::left("controls")
            .resizable(false)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("WFC Visualizer");

                ui.separator();

                ui.label(format!("status: {}", self.status));
                ui.label(format!(
                    "collapsed: {}/{}",
                    self.wfc
                        .cells
                        .iter()
                        .filter(|c| c.count_ones() == 1)
                        .count(),
                    self.wfc.cells.len()
                ));

                if self.finished {
                    ui.colored_label(Color32::LIGHT_GREEN, "finished");
                }

                if self.contradiction {
                    ui.colored_label(Color32::LIGHT_RED, "contradiction");
                }

                ui.separator();

                if ui.button("Step").clicked() {
                    self.step_once();
                }

                ui.checkbox(&mut self.auto, "Auto");

                ui.add(
                    egui::Slider::new(&mut self.steps_per_frame, 1..=200)
                        .text("steps / frame"),
                );

                if ui.button("Run to end").clicked() {
                    self.run_to_end();
                }

                if ui.button("Reset").clicked() {
                    self.reset_world();
                }

                ui.separator();

                let mut should_reset = false;

                should_reset |= ui
                    .add(egui::Slider::new(&mut self.width, 8..=160).text("width"))
                    .changed();

                should_reset |= ui
                    .add(egui::Slider::new(&mut self.height, 8..=120).text("height"))
                    .changed();

                should_reset |= ui
                    .add(
                        egui::Slider::new(&mut self.initial_road_count, 0..=64)
                            .text("initial roads"),
                    )
                    .changed();

                should_reset |= ui
                    .add(
                        egui::Slider::new(&mut self.cell_size, 6.0..=32.0)
                            .text("cell size"),
                    )
                    .changed();

                ui.horizontal(|ui| {
                    ui.label("seed");
                    should_reset |= ui
                        .add(egui::DragValue::new(&mut self.seed).speed(1))
                        .changed();
                });

                if should_reset {
                    self.reset_world();
                }

                ui.separator();

                ui.label("legend:");
                ui.label("number = candidate count");
                ui.colored_label(Color32::from_rgb(220, 170, 40), "yellow = collapsed");
                ui.colored_label(Color32::from_rgb(30, 90, 170), "blue = propagated");
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_grid(ui);
                });
        });
    }
}

fn entropy_color(count: u32, max_count: u32) -> Color32 {
    let t = if max_count <= 1 {
        0.0
    } else {
        count as f32 / max_count as f32
    };

    let r = (70.0 + 120.0 * t) as u8;
    let g = (60.0 + 80.0 * (1.0 - t)) as u8;
    let b = (120.0 + 80.0 * t) as u8;

    Color32::from_rgb(r, g, b)
}
