use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Color32, RichText};

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Search Index Visualizer",
        native_options,
        Box::new(|_cc| Ok(Box::new(SearchApp::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::{wasm_bindgen::JsCast as _, web_sys};

    eframe::WebLogger::init(log::LevelFilter::Info).ok();

    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("search-index-visualizer"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing #search-index-visualizer canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(SearchApp::default()))),
            )
            .await
            .expect("failed to start eframe web app");
    });
}

#[derive(Clone, Debug)]
pub struct Document {
    pub id: usize,
    pub title: String,
    pub body: String,
}

impl Document {
    pub fn new(id: usize, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            body: body.into(),
        }
    }

    fn searchable_text(&self) -> String {
        format!("{} {}", self.title, self.body)
    }
}

#[derive(Clone, Debug)]
pub struct Posting {
    pub doc_index: usize,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub document: Document,
    pub score: f64,
    pub terms: Vec<TermContribution>,
}

#[derive(Clone, Debug)]
pub struct TermContribution {
    pub term: String,
    pub query_weight: f64,
    pub document_weight: f64,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct Analyzer {
    cjk_ngram: usize,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self { cjk_ngram: 2 }
    }
}

impl Analyzer {
    pub fn new(cjk_ngram: usize) -> Self {
        Self {
            cjk_ngram: cjk_ngram.max(1),
        }
    }

    pub fn analyze(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut ascii_buf = String::new();
        let mut cjk_buf = String::new();

        for c in text.chars().flat_map(|c| c.to_lowercase()) {
            if c.is_ascii_alphanumeric() {
                Self::flush_cjk(&mut tokens, &mut cjk_buf, self.cjk_ngram);
                ascii_buf.push(c);
            } else if Self::is_cjk_or_japanese(c) {
                Self::flush_ascii(&mut tokens, &mut ascii_buf);
                cjk_buf.push(c);
            } else {
                Self::flush_ascii(&mut tokens, &mut ascii_buf);
                Self::flush_cjk(&mut tokens, &mut cjk_buf, self.cjk_ngram);
            }
        }

        Self::flush_ascii(&mut tokens, &mut ascii_buf);
        Self::flush_cjk(&mut tokens, &mut cjk_buf, self.cjk_ngram);

        tokens
    }

    fn flush_ascii(tokens: &mut Vec<String>, buf: &mut String) {
        if !buf.is_empty() {
            tokens.push(std::mem::take(buf));
        }
    }

    fn flush_cjk(tokens: &mut Vec<String>, buf: &mut String, n: usize) {
        if buf.is_empty() {
            return;
        }

        let chars: Vec<char> = buf.chars().collect();

        if chars.len() <= n {
            tokens.push(chars.iter().copied().collect::<String>());
        } else {
            tokens.push(chars.iter().copied().collect::<String>());

            for window in chars.windows(n) {
                tokens.push(window.iter().copied().collect::<String>());
            }
        }

        buf.clear();
    }

    fn is_cjk_or_japanese(c: char) -> bool {
        matches!(
            c as u32,
            0x3040..=0x309F
                | 0x30A0..=0x30FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
        )
    }
}

pub struct InvertedIndex {
    docs: Vec<Document>,
    analyzer: Analyzer,
    postings: HashMap<String, Vec<Posting>>,
    idf: HashMap<String, f64>,
}

impl InvertedIndex {
    pub fn build(docs: Vec<Document>, analyzer: Analyzer) -> Self {
        let mut per_doc_tf: Vec<HashMap<String, usize>> = Vec::with_capacity(docs.len());
        let mut document_frequency: HashMap<String, usize> = HashMap::new();

        for doc in &docs {
            let terms = analyzer.analyze(&doc.searchable_text());
            let tf = count_terms(terms);
            let unique_terms: HashSet<String> = tf.keys().cloned().collect();

            for term in unique_terms {
                *document_frequency.entry(term).or_insert(0) += 1;
            }

            per_doc_tf.push(tf);
        }

        let n_docs = docs.len() as f64;
        let idf: HashMap<String, f64> = document_frequency
            .into_iter()
            .map(|(term, df)| {
                let df = df as f64;
                let value = ((n_docs + 1.0) / (df + 1.0)).ln() + 1.0;
                (term, value)
            })
            .collect();

        let mut postings: HashMap<String, Vec<Posting>> = HashMap::new();

        for (doc_index, tf) in per_doc_tf.iter().enumerate() {
            let mut raw_weights = Vec::with_capacity(tf.len());
            let mut norm_sq = 0.0;

            for (term, count) in tf {
                let Some(idf_value) = idf.get(term) else {
                    continue;
                };

                let weight = Self::tf_weight(*count) * *idf_value;
                norm_sq += weight * weight;
                raw_weights.push((term.clone(), weight));
            }

            let norm = norm_sq.sqrt();

            if norm == 0.0 {
                continue;
            }

            for (term, weight) in raw_weights {
                postings.entry(term).or_default().push(Posting {
                    doc_index,
                    weight: weight / norm,
                });
            }
        }

        Self {
            docs,
            analyzer,
            postings,
            idf,
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query_tf = count_terms(self.analyzer.analyze(query));

        if query_tf.is_empty() {
            return Vec::new();
        }

        let mut query_raw_weights = Vec::new();
        let mut query_norm_sq = 0.0;

        for (term, count) in &query_tf {
            let Some(idf_value) = self.idf.get(term) else {
                continue;
            };

            let weight = Self::tf_weight(*count) * *idf_value;
            query_norm_sq += weight * weight;
            query_raw_weights.push((term.clone(), weight));
        }

        let query_norm = query_norm_sq.sqrt();

        if query_norm == 0.0 {
            return Vec::new();
        }

        let mut contributions: HashMap<usize, Vec<TermContribution>> = HashMap::new();

        for (term, raw_weight) in query_raw_weights {
            let query_weight = raw_weight / query_norm;

            if let Some(posting_list) = self.postings.get(&term) {
                for posting in posting_list {
                    let score = query_weight * posting.weight;
                    contributions
                        .entry(posting.doc_index)
                        .or_default()
                        .push(TermContribution {
                            term: term.clone(),
                            query_weight,
                            document_weight: posting.weight,
                            score,
                        });
                }
            }
        }

        let mut results: Vec<SearchResult> = contributions
            .into_iter()
            .filter_map(|(doc_index, mut terms)| {
                let score = terms.iter().map(|term| term.score).sum::<f64>();

                if score <= 0.0 {
                    return None;
                }

                terms.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.term.cmp(&b.term))
                });

                Some(SearchResult {
                    document: self.docs[doc_index].clone(),
                    score,
                    terms,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.document.id.cmp(&b.document.id))
        });

        results.truncate(limit);
        results
    }

    fn idf_rows(&self) -> Vec<(&String, &f64, usize)> {
        let mut rows = self
            .idf
            .iter()
            .map(|(term, idf)| {
                let df = self.postings.get(term).map_or(0, Vec::len);
                (term, idf, df)
            })
            .collect::<Vec<_>>();

        rows.sort_by(|a, b| {
            b.1.partial_cmp(a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });
        rows
    }

    fn tf_weight(count: usize) -> f64 {
        1.0 + (count as f64).ln()
    }
}

fn count_terms(terms: Vec<String>) -> HashMap<String, usize> {
    let mut counts = HashMap::new();

    for term in terms {
        *counts.entry(term).or_insert(0) += 1;
    }

    counts
}

struct SearchApp {
    documents: Vec<Document>,
    analyzer: Analyzer,
    query: String,
    limit: usize,
    selected_result: usize,
    selected_doc: usize,
    index: InvertedIndex,
}

impl Default for SearchApp {
    fn default() -> Self {
        let documents = sample_documents();
        let analyzer = Analyzer::default();
        let index = InvertedIndex::build(documents.clone(), analyzer.clone());

        Self {
            documents,
            analyzer,
            query: "rust 検索".to_string(),
            limit: 6,
            selected_result: 0,
            selected_doc: 0,
            index,
        }
    }
}

impl SearchApp {
    fn rebuild_index(&mut self) {
        self.index = InvertedIndex::build(self.documents.clone(), self.analyzer.clone());
        self.selected_result = 0;
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Search Index");
        ui.separator();

        ui.label("query");
        let query_changed = ui
            .add(
                egui::TextEdit::multiline(&mut self.query)
                    .desired_rows(3)
                    .hint_text("rust 検索"),
            )
            .changed();

        let mut should_rebuild = false;
        should_rebuild |= ui
            .add(egui::Slider::new(&mut self.analyzer.cjk_ngram, 1..=4).text("CJK n-gram"))
            .changed();

        ui.add(egui::Slider::new(&mut self.limit, 1..=10).text("result limit"));

        if should_rebuild {
            self.rebuild_index();
        }

        if query_changed {
            self.selected_result = 0;
        }

        ui.separator();
        ui.label(format!("documents: {}", self.documents.len()));
        ui.label(format!("terms: {}", self.index.idf.len()));
        ui.label(format!(
            "postings: {}",
            self.index.postings.values().map(Vec::len).sum::<usize>()
        ));

        ui.separator();
        ui.label("query tokens");
        let tokens = self.analyzer.analyze(&self.query);
        draw_token_flow(ui, &tokens);
    }

    fn draw_results(&mut self, ui: &mut egui::Ui, results: &[SearchResult]) {
        ui.heading("Ranking");
        ui.separator();

        if results.is_empty() {
            ui.colored_label(Color32::LIGHT_RED, "no matching documents");
            return;
        }

        let max_score = results
            .first()
            .map_or(1.0, |result| result.score)
            .max(0.001);

        for (i, result) in results.iter().enumerate() {
            let selected = self.selected_result == i;
            let label = format!(
                "#{:02}  {:.4}  {}",
                i + 1,
                result.score,
                result.document.title
            );

            if ui.selectable_label(selected, label).clicked() {
                self.selected_result = i;
            }

            let fill = if selected {
                Color32::from_rgb(78, 126, 190)
            } else {
                Color32::from_rgb(63, 83, 108)
            };
            ui.add(
                egui::ProgressBar::new((result.score / max_score) as f32)
                    .fill(fill)
                    .show_percentage(),
            );
            ui.add_space(8.0);
        }
    }

    fn draw_detail(&self, ui: &mut egui::Ui, results: &[SearchResult]) {
        ui.heading("Score Detail");
        ui.separator();

        let Some(result) = results.get(self.selected_result) else {
            return;
        };

        ui.label(RichText::new(&result.document.title).strong());
        ui.label(format!("id: {}", result.document.id));
        ui.label(format!("score: {:.6}", result.score));
        ui.add_space(6.0);
        ui.label(&result.document.body);

        ui.separator();
        ui.label("term contributions");

        egui::Grid::new("contributions")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                ui.strong("term");
                ui.strong("query");
                ui.strong("doc");
                ui.strong("score");
                ui.end_row();

                for term in result.terms.iter().take(12) {
                    ui.label(&term.term);
                    ui.label(format!("{:.3}", term.query_weight));
                    ui.label(format!("{:.3}", term.document_weight));
                    ui.label(format!("{:.4}", term.score));
                    ui.end_row();
                }
            });
    }

    fn draw_documents(&mut self, ui: &mut egui::Ui) {
        ui.heading("Documents");
        ui.separator();

        for (i, doc) in self.documents.iter().enumerate() {
            let selected = self.selected_doc == i;
            if ui
                .selectable_label(selected, format!("{}: {}", doc.id, doc.title))
                .clicked()
            {
                self.selected_doc = i;
            }
        }

        ui.separator();

        let doc = &self.documents[self.selected_doc];
        ui.label(RichText::new(&doc.title).strong());
        ui.label(&doc.body);
        ui.add_space(6.0);
        ui.label("analyzed terms");
        draw_token_flow(ui, &self.analyzer.analyze(&doc.searchable_text()));
    }

    fn draw_dictionary(&self, ui: &mut egui::Ui) {
        ui.heading("Dictionary");
        ui.separator();

        egui::Grid::new("dictionary")
            .striped(true)
            .num_columns(3)
            .show(ui, |ui| {
                ui.strong("term");
                ui.strong("df");
                ui.strong("idf");
                ui.end_row();

                for (term, idf, df) in self.index.idf_rows().into_iter().take(40) {
                    ui.label(term);
                    ui.label(df.to_string());
                    ui.label(format!("{idf:.3}"));
                    ui.end_row();
                }
            });
    }
}

impl eframe::App for SearchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let results = self.index.search(&self.query, self.limit);

        if self.selected_result >= results.len() {
            self.selected_result = 0;
        }

        egui::SidePanel::left("controls")
            .resizable(false)
            .default_width(280.0)
            .show(ctx, |ui| self.draw_controls(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                    self.draw_results(ui, &results);
                    ui.add_space(18.0);
                    self.draw_detail(ui, &results);
                });

                egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                    self.draw_documents(ui);
                    ui.add_space(18.0);
                    self.draw_dictionary(ui);
                });
            });
        });
    }
}

fn draw_token_flow(ui: &mut egui::Ui, tokens: &[String]) {
    ui.horizontal_wrapped(|ui| {
        for token in tokens {
            ui.label(
                RichText::new(token)
                    .monospace()
                    .background_color(Color32::from_rgb(45, 55, 70)),
            );
        }
    });
}

fn sample_documents() -> Vec<Document> {
    vec![
        Document::new(
            1,
            "Rustで作る小さな検索エンジン",
            "転置インデックスとTF-IDFとコサイン類似度で文書をランキングする。",
        ),
        Document::new(
            2,
            "RustのWebフレームワーク比較",
            "axumやactix-webを使ってHTTP APIを実装する。",
        ),
        Document::new(
            3,
            "家系図グラフの可視化",
            "人物と親子関係をグラフとして扱い、eguiで表示する。",
        ),
        Document::new(
            4,
            "全文検索の基礎",
            "文書をベクトル空間に写像し、クエリとの内積で適合度を計算する。",
        ),
        Document::new(
            5,
            "英語と日本語のトークン化",
            "ASCII wordと日本語n-gramを同じ転置インデックスに登録する。",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_returns_relevant_document() {
        let docs = vec![
            Document::new(1, "Rust検索", "転置インデックスで検索する"),
            Document::new(2, "料理メモ", "カレーを作る"),
        ];

        let index = InvertedIndex::build(docs, Analyzer::default());
        let results = index.search("rust 検索", 10);

        assert_eq!(results[0].document.id, 1);
    }

    #[test]
    fn cjk_text_is_ngrammed() {
        let analyzer = Analyzer::new(2);
        let terms = analyzer.analyze("全文検索");

        assert!(terms.iter().any(|term| term == "全文検索"));
        assert!(terms.iter().any(|term| term == "全文"));
        assert!(terms.iter().any(|term| term == "検索"));
    }
}
