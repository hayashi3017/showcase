// ─── Raft 合意アルゴリズム egui ビジュアライザ ────────────────────────────────
//
// Raft は分散システムにおける合意（consensus）アルゴリズムです。
// Paxos と同等の安全性を保ちながら「理解しやすさ」を設計目標とした論文
// "In Search of an Understandable Consensus Algorithm" (Ongaro & Ousterhout, 2014)
// をベースにしています。
//
// このモジュールは以下の 3 つの中心的サブプロトコルをシミュレートします。
//
//  1. リーダー選出 (Leader Election)
//     ─ Follower がタイムアウトすると Candidate に昇格し，多数決投票で Leader を決定。
//
//  2. ログ複製 (Log Replication)
//     ─ Leader がクライアント命令を受け取り AppendEntries RPC で全 Follower へ複製。
//
//  3. コミット・適用 (Commit / Apply)
//     ─ 過半数からの Ack を受けたエントリを commit し，ステートマシンに適用。
//
// ─── ファイル構成 ──────────────────────────────────────────────────────────────
//  Role / LogEntry / Rpc / Envelope  ── データ型定義
//  Node                              ── 単一ノードの状態
//  Simulation                        ── クラスタ全体のシミュレーションロジック
//  RaftApp / impl eframe::App        ── GUI レンダリング
// ──────────────────────────────────────────────────────────────────────────────

use std::collections::{BTreeMap, BTreeSet, VecDeque};

// web_time::Instant は native では std::time::Instant と同等で、
// wasm32 では performance.now() を使うクロスプラットフォーム実装。
use web_time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

// ネイティブ版エントリポイント（wasm32 ではコンパイル対象外）。
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 820.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Raft egui Visualizer",
        options,
        Box::new(|_cc| Ok(Box::new(RaftApp::default()))),
    )
}

// wasm32 版エントリポイント。Trunk が wasm-bindgen でバインドする。
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::{wasm_bindgen::JsCast as _, web_sys};

    eframe::WebLogger::init(log::LevelFilter::Info).ok();

    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("raft-consensus-viz"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing #raft-consensus-viz canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(RaftApp::default()))),
            )
            .await
            .expect("failed to start eframe web app");
    });
}

// ─── ノードの役割 ──────────────────────────────────────────────────────────────
//
// Raft では各ノードが常に次の 3 役割のいずれか 1 つを持ちます。
//
//  Follower  : 通常状態。Leader からの AppendEntries を待つ。
//              タイムアウトすると Candidate へ昇格。
//
//  Candidate : 選挙中。RequestVote を全 Peer へ送り，過半数を得れば Leader になる。
//
//  Leader    : 1 つの term に最大 1 人。クライアント要求を受け付け，
//              ハートビート（空の AppendEntries）で Follower のタイマーをリセットする。
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Follower,
    Candidate,
    Leader,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Follower => "Follower",
            Role::Candidate => "Candidate",
            Role::Leader => "Leader",
        }
    }
}

// ─── ログエントリ ──────────────────────────────────────────────────────────────
//
// Raft のログは単調増加するエントリの列です。
//  term    : エントリが作成されたときの Leader の任期番号。
//            同じ term・同じ index のエントリは全ノードで同一であることが保証される
//            (Log Matching Property)。
//  command : ステートマシンへ渡す命令文字列（ここでは "counter += N"）。
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct LogEntry {
    term: u64,
    command: String,
}

// ─── RPC メッセージ型 ──────────────────────────────────────────────────────────
//
// Raft が使用する 2 種の RPC とそのレスポンスを enum で表現します。
//
//  RequestVote          : Candidate が選挙期間中に全 Peer へ送る投票依頼。
//  RequestVoteResponse  : vote_granted=true なら票を投じる。
//
//  AppendEntries        : Leader が送るログ複製 / ハートビート RPC。
//                         entries が空のときはハートビートとして機能する。
//  AppendEntriesResponse: 複製成功/失敗と，Follower 側の match_index を返す。
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Rpc {
    RequestVote {
        term: u64,
        candidate_id: usize,
        // Candidate のログの「最後のエントリ」情報。
        // Follower より新しいログを持つ Candidate にのみ票を入れる（Election Restriction）。
        last_log_index: usize,
        last_log_term: u64,
    },
    RequestVoteResponse {
        term: u64,
        vote_granted: bool,
        from: usize,
    },
    AppendEntries {
        term: u64,
        leader_id: usize,
        // entries[0] の直前のエントリ。Follower がログの連続性を確認するために使う。
        prev_log_index: usize,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        // Leader がコミット済みとみなす最大インデックス。
        // Follower はこれを元に自分の commit_index を進める。
        leader_commit: usize,
    },
    AppendEntriesResponse {
        term: u64,
        success: bool,
        from: usize,
        // Follower が一致を確認できた最後のログインデックス。
        // Leader はこの値で next_index / match_index を更新する。
        match_index: usize,
    },
}

impl Rpc {
    fn short_label(&self) -> &'static str {
        match self {
            Rpc::RequestVote { .. } => "Vote?",
            Rpc::RequestVoteResponse { vote_granted, .. } => {
                if *vote_granted {
                    "Yes"
                } else {
                    "No"
                }
            }
            Rpc::AppendEntries { entries, .. } => {
                if entries.is_empty() {
                    "Beat"
                } else {
                    "Append"
                }
            }
            Rpc::AppendEntriesResponse { success, .. } => {
                if *success {
                    "Ack"
                } else {
                    "Reject"
                }
            }
        }
    }
}

// ─── メッセージエンベロープ ────────────────────────────────────────────────────
//
// 実ネットワークの非同期性をシミュレートするため，メッセージに
// 「送信時刻」と「配送予定時刻」を付加して遅延をモデル化します。
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Envelope {
    from: usize,
    to: usize,
    sent_at: f32,
    deliver_at: f32,
    rpc: Rpc,
}

// ─── ノード状態 ───────────────────────────────────────────────────────────────
//
// Raft 論文 Figure 2 で定義されている永続・揮発性状態を一構造体に収めています。
//
// 永続状態（実装ではインメモリ）
//  current_term : 現在の任期番号。単調増加し，再起動後も保存が必要（ここでは省略）。
//  voted_for    : 現在 term で票を入れた Candidate ID。
//  log          : ログエントリ列。
//
// 揮発性状態（全ノード共通）
//  commit_index : コミット済み最大インデックス。
//  last_applied : ステートマシンへ適用済み最大インデックス。
//
// 揮発性状態（Leader のみ）
//  next_index   : 各 Follower へ次に送るべきログインデックス。
//  match_index  : 各 Follower との一致が確認できた最大インデックス。
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Node {
    id: usize,
    role: Role,
    current_term: u64,
    voted_for: Option<usize>,
    log: Vec<LogEntry>,
    commit_index: usize,
    last_applied: usize,
    // "counter += N" コマンドを適用する簡易ステートマシン。
    state_machine: BTreeMap<String, i64>,

    // Follower / Candidate 用タイマー。election_timeout を超えると選挙を開始する。
    election_elapsed: f32,
    election_timeout: f32,
    // Leader 用タイマー。heartbeat_interval ごとにハートビートを送る。
    heartbeat_elapsed: f32,

    // Candidate として集めた票の送信元集合。過半数に達したら Leader になる。
    votes_received: BTreeSet<usize>,
    // Leader 専用（上記参照）。
    next_index: Vec<usize>,
    match_index: Vec<usize>,

    // クラッシュ / 再起動をシミュレートするフラグ。
    alive: bool,
}

impl Node {
    fn new(id: usize, node_count: usize, election_timeout: f32) -> Self {
        Self {
            id,
            role: Role::Follower,
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
            state_machine: BTreeMap::new(),
            election_elapsed: 0.0,
            election_timeout,
            heartbeat_elapsed: 0.0,
            votes_received: BTreeSet::new(),
            next_index: vec![1; node_count],
            match_index: vec![0; node_count],
            alive: true,
        }
    }

    fn last_log_index(&self) -> usize {
        self.log.len()
    }

    fn last_log_term(&self) -> u64 {
        self.log.last().map(|entry| entry.term).unwrap_or(0)
    }
}

// ─── シミュレーション ──────────────────────────────────────────────────────────
//
// Simulation はクラスタ全体の状態機械です。
//  tick()           : フレームごとに呼ばれ，タイマー進行・メッセージ配送を行う。
//  send()           : 遅延・パケットロスを加えてメッセージをキューに積む。
//  start_election() : Candidate への昇格と RequestVote 送信。
//  become_leader()  : Leader への昇格と初回 AppendEntries 送信。
//  deliver()        : メッセージを対象ノードのハンドラへ振り分ける。
// ─────────────────────────────────────────────────────────────────────────────

struct Simulation {
    nodes: Vec<Node>,
    // 配送待ちメッセージのキュー。deliver_at 時刻に配送される。
    messages: Vec<Envelope>,
    time: f32,
    speed: f32,
    paused: bool,
    drop_rate: f32,
    min_delay: f32,
    max_delay: f32,
    heartbeat_interval: f32,
    // 暗号用途なし・外部クレート非依存の LCG 乱数状態。
    rng_state: u64,
    client_seq: u64,
    // UI 用イベントログ（最新が先頭）。
    events: VecDeque<String>,
    selected_node: Option<usize>,
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new(5)
    }
}

impl Simulation {
    fn new(node_count: usize) -> Self {
        let mut sim = Self {
            nodes: Vec::new(),
            messages: Vec::new(),
            time: 0.0,
            speed: 1.0,
            paused: false,
            drop_rate: 0.0,
            min_delay: 0.10,
            max_delay: 0.75,
            heartbeat_interval: 0.55,
            rng_state: 0x5EED_1234_ABCD_0001,
            client_seq: 0,
            events: VecDeque::new(),
            selected_node: None,
        };

        for id in 0..node_count {
            let timeout = sim.random_election_timeout();
            sim.nodes.push(Node::new(id, node_count, timeout));
        }

        sim.event("cluster started".to_owned());
        sim
    }

    // 過半数（クォーラム）。n=5 なら 3。
    fn majority(&self) -> usize {
        self.nodes.len() / 2 + 1
    }

    fn event(&mut self, text: String) {
        self.events.push_front(format!("{:>6.2}s  {}", self.time, text));
        while self.events.len() > 160 {
            self.events.pop_back();
        }
    }

    // 外部 rand クレートを避けるための簡易 LCG。
    // シミュレーション用途なので暗号学的な安全性は不要。
    fn rand_u64(&mut self) -> u64 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.rng_state
    }

    fn rand_f32(&mut self) -> f32 {
        let x = (self.rand_u64() >> 40) as u32;
        x as f32 / ((1u32 << 24) as f32)
    }

    // 選挙タイムアウトは 1.65〜3.85 秒のランダム値。
    // ばらつきをつけることで，複数 Candidate が同時に選挙を始める split vote を抑制する。
    fn random_election_timeout(&mut self) -> f32 {
        1.65 + self.rand_f32() * 2.2
    }

    fn random_delay(&mut self) -> f32 {
        self.min_delay + self.rand_f32() * (self.max_delay - self.min_delay).max(0.01)
    }

    // ─── メインループ ─────────────────────────────────────────────────────────
    //
    // 1. 仮想時刻を real_dt * speed だけ進める。
    // 2. deliver_at を超えたメッセージを配送する。
    // 3. 各ノードのタイマーを進め，タイムアウトしたノードの選挙・ハートビートを起動。
    // 4. コミット済みエントリをステートマシンへ適用する。
    // ─────────────────────────────────────────────────────────────────────────

    fn tick(&mut self, real_dt: f32) {
        let dt = real_dt * self.speed;
        self.time += dt;

        // ── メッセージ配送 ──────────────────────────────────────────────────
        let mut ready = Vec::new();
        let mut pending = Vec::new();

        for msg in self.messages.drain(..) {
            if msg.deliver_at <= self.time {
                ready.push(msg);
            } else {
                pending.push(msg);
            }
        }

        self.messages = pending;

        for msg in ready {
            self.deliver(msg);
        }

        // ── タイマー進行 ────────────────────────────────────────────────────
        // elections / heartbeats を後でまとめて処理する理由:
        // ループ中に self を借用したまま start_election() を呼べないため，
        // 対象 ID をリストに溜めてからまとめて処理する。
        let mut elections = Vec::new();
        let mut heartbeats = Vec::new();

        for id in 0..self.nodes.len() {
            if !self.nodes[id].alive {
                continue;
            }

            match self.nodes[id].role {
                Role::Leader => {
                    self.nodes[id].heartbeat_elapsed += dt;

                    if self.nodes[id].heartbeat_elapsed >= self.heartbeat_interval {
                        self.nodes[id].heartbeat_elapsed = 0.0;
                        heartbeats.push(id);
                    }
                }
                Role::Follower | Role::Candidate => {
                    self.nodes[id].election_elapsed += dt;

                    if self.nodes[id].election_elapsed >= self.nodes[id].election_timeout {
                        elections.push(id);
                    }
                }
            }
        }

        for id in elections {
            if self.nodes[id].alive && self.nodes[id].role != Role::Leader {
                self.start_election(id);
            }
        }

        for id in heartbeats {
            if self.nodes[id].alive && self.nodes[id].role == Role::Leader {
                self.send_append_entries_to_all(id);
            }
        }

        for id in 0..self.nodes.len() {
            self.apply_committed_entries(id);
        }
    }

    // ─── メッセージ送信 ───────────────────────────────────────────────────────
    //
    // drop_rate の確率でメッセージを破棄することでネットワーク障害を模倣する。
    // 配送遅延は [min_delay, max_delay] の一様乱数で決める。
    // ─────────────────────────────────────────────────────────────────────────

    fn send(&mut self, from: usize, to: usize, rpc: Rpc) {
        if from == to {
            return;
        }

        if self.rand_f32() < self.drop_rate {
            self.event(format!("drop {}: S{} -> S{}", rpc.short_label(), from, to));
            return;
        }

        let delay = self.random_delay();

        self.messages.push(Envelope {
            from,
            to,
            sent_at: self.time,
            deliver_at: self.time + delay,
            rpc,
        });
    }

    fn reset_election_timer(&mut self, id: usize) {
        let timeout = self.random_election_timeout();
        self.nodes[id].election_elapsed = 0.0;
        self.nodes[id].election_timeout = timeout;
    }

    fn become_follower(&mut self, id: usize, term: u64) {
        let timeout = self.random_election_timeout();

        let node = &mut self.nodes[id];
        node.role = Role::Follower;
        node.current_term = term;
        // より大きい term を見たら即座に voted_for をクリアする。
        // これにより，同じ term への二重投票が防止される。
        node.voted_for = None;
        node.votes_received.clear();
        node.election_elapsed = 0.0;
        node.election_timeout = timeout;
        node.heartbeat_elapsed = 0.0;
    }

    // ─── リーダー選出 ─────────────────────────────────────────────────────────
    //
    // 選挙開始条件: election_timeout 経過後も Leader からハートビートが届かなかった。
    //
    // 手順:
    //  1. term をインクリメントして Candidate になる。
    //  2. 自分自身に票を入れる（votes_received に自 ID を追加）。
    //  3. 全 Peer へ RequestVote を送る。
    //  4. 過半数の票を得たら become_leader() を呼ぶ。
    //
    // 単一ノードのクラスタでは即座に Leader になる。
    // ─────────────────────────────────────────────────────────────────────────

    fn start_election(&mut self, candidate_id: usize) {
        let timeout = self.random_election_timeout();

        let term;
        let last_log_index;
        let last_log_term;

        {
            let node = &mut self.nodes[candidate_id];

            node.role = Role::Candidate;
            node.current_term += 1;
            node.voted_for = Some(candidate_id);
            node.votes_received.clear();
            node.votes_received.insert(candidate_id);
            node.election_elapsed = 0.0;
            node.election_timeout = timeout;

            term = node.current_term;
            last_log_index = node.last_log_index();
            last_log_term = node.last_log_term();
        }

        self.event(format!("S{} starts election for term {}", candidate_id, term));

        if self.majority() == 1 {
            self.become_leader(candidate_id);
            return;
        }

        for peer in 0..self.nodes.len() {
            if peer == candidate_id {
                continue;
            }

            self.send(
                candidate_id,
                peer,
                Rpc::RequestVote {
                    term,
                    candidate_id,
                    last_log_index,
                    last_log_term,
                },
            );
        }
    }

    fn become_leader(&mut self, leader_id: usize) {
        // next_index の初期値は「自分のログ末尾 + 1」（論文 §5.3）。
        // match_index の初期値は 0（未確認）。
        let last = self.nodes[leader_id].log.len() + 1;
        let term = self.nodes[leader_id].current_term;
        let node_count = self.nodes.len();

        {
            let node = &mut self.nodes[leader_id];

            node.role = Role::Leader;
            node.voted_for = Some(leader_id);
            node.next_index = vec![last; node_count];
            node.match_index = vec![0; node_count];
            // 自分自身のログは既に一致している。
            node.match_index[leader_id] = node.log.len();
            node.heartbeat_elapsed = 0.0;
            node.election_elapsed = 0.0;
        }

        self.event(format!("S{} becomes LEADER for term {}", leader_id, term));
        // Leader になった直後に全 Follower へ AppendEntries を送り，
        // 自分の権威を確立すると同時に古いログを正す。
        self.send_append_entries_to_all(leader_id);
    }

    // ─── メッセージ配送ディスパッチャ ────────────────────────────────────────

    fn deliver(&mut self, msg: Envelope) {
        if msg.to >= self.nodes.len() || !self.nodes[msg.to].alive {
            return;
        }

        match msg.rpc {
            Rpc::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => self.on_request_vote(
                msg.to,
                msg.from,
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            ),

            Rpc::RequestVoteResponse {
                term,
                vote_granted,
                from,
            } => self.on_request_vote_response(msg.to, term, vote_granted, from),

            Rpc::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => self.on_append_entries(
                msg.to,
                msg.from,
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            ),

            Rpc::AppendEntriesResponse {
                term,
                success,
                from,
                match_index,
            } => self.on_append_entries_response(msg.to, term, success, from, match_index),
        }
    }

    // ─── RequestVote ハンドラ ─────────────────────────────────────────────────
    //
    // 投票条件（Raft 論文 §5.2, §5.4）:
    //  A. Candidate の term >= 自分の current_term
    //  B. 自分がまだ誰にも票を入れていない，または同じ Candidate に入れている
    //  C. Candidate のログが自分のログより「新しい」（Election Restriction）
    //     ─ last_log_term が大きい，または同じなら last_log_index が大きい等。
    //
    // 全条件を満たした場合のみ vote_granted=true で返す。
    // ─────────────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn on_request_vote(
        &mut self,
        receiver_id: usize,
        reply_to: usize,
        term: u64,
        candidate_id: usize,
        last_log_index: usize,
        last_log_term: u64,
    ) {
        // より大きい term を見たら即座に Follower に戻る（§5.1 ルール）。
        if term > self.nodes[receiver_id].current_term {
            self.become_follower(receiver_id, term);
        }

        let mut vote_granted = false;
        let response_term;

        {
            let node = &mut self.nodes[receiver_id];
            response_term = node.current_term;

            if term == node.current_term {
                // Election Restriction: Candidate のログが少なくとも自分と同等に新しいこと。
                let up_to_date = last_log_term > node.last_log_term()
                    || (last_log_term == node.last_log_term()
                        && last_log_index >= node.last_log_index());

                let can_vote = node.voted_for.is_none() || node.voted_for == Some(candidate_id);

                if can_vote && up_to_date {
                    node.voted_for = Some(candidate_id);
                    vote_granted = true;
                }
            }
        }

        if vote_granted {
            // 票を入れたら自分の選挙タイマーをリセットして二重選挙を防ぐ。
            self.reset_election_timer(receiver_id);
            self.event(format!("S{} votes for S{}", receiver_id, candidate_id));
        }

        self.send(
            receiver_id,
            reply_to,
            Rpc::RequestVoteResponse {
                term: response_term,
                vote_granted,
                from: receiver_id,
            },
        );
    }

    // ─── RequestVoteResponse ハンドラ ─────────────────────────────────────────
    //
    // 票を受け取った Candidate が過半数を確認したら Leader になる。
    // 古い term のレスポンスは無視（ネットワーク遅延による重複を排除）。
    // ─────────────────────────────────────────────────────────────────────────

    fn on_request_vote_response(
        &mut self,
        candidate_id: usize,
        term: u64,
        vote_granted: bool,
        from: usize,
    ) {
        if term > self.nodes[candidate_id].current_term {
            self.become_follower(candidate_id, term);
            return;
        }

        if self.nodes[candidate_id].role != Role::Candidate {
            return;
        }

        // 自分の current_term と異なる term のレスポンスは古いので無視。
        if term != self.nodes[candidate_id].current_term {
            return;
        }

        if vote_granted {
            self.nodes[candidate_id].votes_received.insert(from);

            let votes = self.nodes[candidate_id].votes_received.len();

            if votes >= self.majority() {
                self.become_leader(candidate_id);
            }
        }
    }

    // ─── AppendEntries ハンドラ ───────────────────────────────────────────────
    //
    // ログ複製の中核。Follower 側の処理:
    //  1. 古い term の Leader は拒否。
    //  2. prev_log_index / prev_log_term でログの一貫性を確認（Consistency Check）。
    //  3. 競合するエントリがあれば截断して上書き。
    //  4. leader_commit を見て commit_index を進める。
    // ─────────────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn on_append_entries(
        &mut self,
        follower_id: usize,
        reply_to: usize,
        term: u64,
        _leader_id: usize,
        prev_log_index: usize,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        leader_commit: usize,
    ) {
        // 古い Leader からのメッセージは拒否（stale leader の排除）。
        if term < self.nodes[follower_id].current_term {
            let current_term = self.nodes[follower_id].current_term;
            let match_index = self.nodes[follower_id].log.len();

            self.send(
                follower_id,
                reply_to,
                Rpc::AppendEntriesResponse {
                    term: current_term,
                    success: false,
                    from: follower_id,
                    match_index,
                },
            );

            return;
        }

        if term > self.nodes[follower_id].current_term
            || self.nodes[follower_id].role != Role::Follower
        {
            // 新しい term を持つ Leader を認めて Follower に降格。
            self.become_follower(follower_id, term);
        } else {
            // 同じ term なら選挙タイマーをリセットするだけ（ハートビート受信）。
            self.reset_election_timer(follower_id);
        }

        let mut success = false;
        let mut matched = self.nodes[follower_id].log.len();

        {
            let node = &mut self.nodes[follower_id];

            // Consistency Check: prev_log_index==0 は「ログが空」を意味する特別ケース。
            let prev_ok = prev_log_index == 0
                || (prev_log_index <= node.log.len()
                    && node.log[prev_log_index - 1].term == prev_log_term);

            if prev_ok {
                success = true;

                let mut insert_at = prev_log_index;

                for entry in entries {
                    if insert_at < node.log.len() {
                        // 既存エントリと term が違う場合のみ截断＆上書き。
                        // 同じ term なら内容も同一なので何もしない（冪等性）。
                        if node.log[insert_at].term != entry.term {
                            node.log.truncate(insert_at);
                            node.log.push(entry);
                        }
                    } else {
                        node.log.push(entry);
                    }

                    insert_at += 1;
                }

                matched = insert_at;

                // Leader のコミットインデックスと自分のログ末尾の小さい方まで進める。
                if leader_commit > node.commit_index {
                    node.commit_index = leader_commit.min(node.log.len());
                }
            }
        }

        let response_term = self.nodes[follower_id].current_term;

        self.send(
            follower_id,
            reply_to,
            Rpc::AppendEntriesResponse {
                term: response_term,
                success,
                from: follower_id,
                match_index: matched,
            },
        );
    }

    // ─── AppendEntriesResponse ハンドラ ──────────────────────────────────────
    //
    // Leader 側の処理:
    //  成功: match_index / next_index を更新し，コミット条件を確認する。
    //  失敗: next_index を 1 減らして再送する（バイナリサーチの線形版）。
    // ─────────────────────────────────────────────────────────────────────────

    fn on_append_entries_response(
        &mut self,
        leader_id: usize,
        term: u64,
        success: bool,
        from: usize,
        match_index: usize,
    ) {
        if term > self.nodes[leader_id].current_term {
            self.become_follower(leader_id, term);
            return;
        }

        if self.nodes[leader_id].role != Role::Leader {
            return;
        }

        if term != self.nodes[leader_id].current_term {
            return;
        }

        if success {
            self.nodes[leader_id].match_index[from] = match_index;
            self.nodes[leader_id].next_index[from] = match_index + 1;
            // Follower のログが一致したら，コミット可能か確認する。
            self.advance_leader_commit(leader_id);
        } else {
            // Consistency Check 失敗 → next_index を 1 戻して再試行。
            if self.nodes[leader_id].next_index[from] > 1 {
                self.nodes[leader_id].next_index[from] -= 1;
            }

            self.send_append_entries(leader_id, from);
        }
    }

    fn send_append_entries_to_all(&mut self, leader_id: usize) {
        for peer in 0..self.nodes.len() {
            if peer != leader_id {
                self.send_append_entries(leader_id, peer);
            }
        }
    }

    // ─── AppendEntries 送信 ───────────────────────────────────────────────────
    //
    // Follower の next_index から始まるエントリを entries に詰めて送る。
    // next_index が 1 のときは全エントリを送る（初期同期）。
    // ─────────────────────────────────────────────────────────────────────────

    fn send_append_entries(&mut self, leader_id: usize, follower_id: usize) {
        if !self.nodes[leader_id].alive {
            return;
        }

        let leader = &self.nodes[leader_id];

        let next = leader.next_index[follower_id]
            .max(1)
            .min(leader.log.len() + 1);

        let prev_log_index = next - 1;

        let prev_log_term = if prev_log_index == 0 {
            0
        } else {
            leader.log[prev_log_index - 1].term
        };

        let entries = leader.log[prev_log_index..].to_vec();
        let term = leader.current_term;
        let leader_commit = leader.commit_index;

        self.send(
            leader_id,
            follower_id,
            Rpc::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            },
        );
    }

    // ─── コミット判定 ─────────────────────────────────────────────────────────
    //
    // Raft の Safety 保証（§5.4.2）:
    //  Leader は「現在 term のエントリ」が過半数に複製されたときのみコミットできる。
    //  過去 term のエントリは現在 term エントリのコミットに引きずられて間接的にコミットされる。
    //  これにより，コミット後に Leader が変わってもコミット済みエントリが失われない。
    // ─────────────────────────────────────────────────────────────────────────

    fn advance_leader_commit(&mut self, leader_id: usize) {
        let term = self.nodes[leader_id].current_term;
        let current_commit = self.nodes[leader_id].commit_index;
        let last_index = self.nodes[leader_id].log.len();

        let mut new_commit = current_commit;

        for index in (current_commit + 1)..=last_index {
            // 過去 term のエントリは直接コミット判定しない（論文 Figure 8 の反例を防ぐ）。
            if self.nodes[leader_id].log[index - 1].term != term {
                continue;
            }

            // Leader 自身の match_index も含めて過半数を数える。
            let replicated = self.nodes[leader_id]
                .match_index
                .iter()
                .filter(|&&m| m >= index)
                .count();

            if replicated >= self.majority() {
                new_commit = index;
            }
        }

        if new_commit > current_commit {
            self.nodes[leader_id].commit_index = new_commit;
            self.event(format!("S{} commits entries <= {}", leader_id, new_commit));
            // コミット後にハートビートを即送信して Follower の commit_index を早期更新。
            self.send_append_entries_to_all(leader_id);
        }
    }

    // ─── ステートマシン適用 ───────────────────────────────────────────────────
    //
    // commit_index と last_applied の差分を順に適用する。
    // Apply は冪等である必要があり（Raft は at-least-once 保証のため），
    // ここでは単純なインクリメントなので問題なし。
    // ─────────────────────────────────────────────────────────────────────────

    fn apply_committed_entries(&mut self, node_id: usize) {
        if !self.nodes[node_id].alive {
            return;
        }

        while self.nodes[node_id].last_applied < self.nodes[node_id].commit_index {
            let entry = self.nodes[node_id].log[self.nodes[node_id].last_applied].clone();

            apply_command(&mut self.nodes[node_id].state_machine, &entry.command);

            self.nodes[node_id].last_applied += 1;
        }
    }

    fn leader_id(&self) -> Option<usize> {
        self.nodes
            .iter()
            .find(|n| n.alive && n.role == Role::Leader)
            .map(|n| n.id)
    }

    // ─── クライアントリクエスト ───────────────────────────────────────────────
    //
    // クライアントは常に Leader に命令を送る。Leader 以外に送っても拒否される。
    // Leader はエントリを自分のログに追加し，即座に AppendEntries で複製を開始する。
    // ─────────────────────────────────────────────────────────────────────────

    fn propose_client_command(&mut self) {
        let Some(leader_id) = self.leader_id() else {
            self.event("client request rejected: no leader".to_owned());
            return;
        };

        self.client_seq += 1;

        let command = format!("counter += {}", self.client_seq);
        let term = self.nodes[leader_id].current_term;

        self.nodes[leader_id].log.push(LogEntry {
            term,
            command: command.clone(),
        });

        let index = self.nodes[leader_id].log.len();

        // Leader は自分自身の match_index を先行更新する。
        self.nodes[leader_id].match_index[leader_id] = index;
        self.nodes[leader_id].next_index[leader_id] = index + 1;

        self.event(format!("client -> S{}: {}", leader_id, command));

        self.send_append_entries_to_all(leader_id);
    }

    // ─── 障害注入 ─────────────────────────────────────────────────────────────
    //
    // alive=false にするとメッセージ配送をスキップし，タイマーも進まない。
    // 再起動時は Follower として同じ term で復帰する（永続化省略のため term は保持）。
    // ─────────────────────────────────────────────────────────────────────────

    fn toggle_node(&mut self, id: usize) {
        if id >= self.nodes.len() {
            return;
        }

        self.nodes[id].alive = !self.nodes[id].alive;

        if self.nodes[id].alive {
            self.become_follower(id, self.nodes[id].current_term);
            self.event(format!("S{} restarted", id));
        } else {
            self.nodes[id].role = Role::Follower;
            self.nodes[id].votes_received.clear();
            self.nodes[id].heartbeat_elapsed = 0.0;
            self.nodes[id].election_elapsed = 0.0;
            self.event(format!("S{} crashed", id));
        }
    }
}

// ─── ステートマシン ───────────────────────────────────────────────────────────
//
// "KEY += VALUE" 形式のコマンドだけを解釈する最小ステートマシン。
// ─────────────────────────────────────────────────────────────────────────────

fn apply_command(state: &mut BTreeMap<String, i64>, command: &str) {
    let parts = command.split_whitespace().collect::<Vec<_>>();

    if parts.len() == 3 && parts[1] == "+=" {
        if let Ok(delta) = parts[2].parse::<i64>() {
            *state.entry(parts[0].to_owned()).or_insert(0) += delta;
        }
    }
}

// ─── GUI アプリケーション ─────────────────────────────────────────────────────

struct RaftApp {
    sim: Simulation,
    last_frame: Instant,
    auto_client: bool,
    auto_client_elapsed: f32,
}

impl Default for RaftApp {
    fn default() -> Self {
        Self {
            sim: Simulation::default(),
            last_frame: Instant::now(),
            auto_client: false,
            auto_client_elapsed: 0.0,
        }
    }
}

impl eframe::App for RaftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let real_dt = (now - self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;

        if !self.sim.paused {
            self.sim.tick(real_dt);

            self.auto_client_elapsed += real_dt * self.sim.speed;

            // auto_client が有効なら約 2.4 秒ごとに命令を送る。
            if self.auto_client && self.auto_client_elapsed >= 2.4 {
                self.auto_client_elapsed = 0.0;
                self.sim.propose_client_command();
            }

            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if self.sim.paused {
                        "▶ resume"
                    } else {
                        "⏸ pause"
                    })
                    .clicked()
                {
                    self.sim.paused = !self.sim.paused;
                }

                if ui.button("step").clicked() {
                    self.sim.tick(0.18);
                }

                if ui.button("client request").clicked() {
                    self.sim.propose_client_command();
                }

                ui.checkbox(&mut self.auto_client, "auto client");

                if ui.button("reset 5 nodes").clicked() {
                    *self = Self::default();
                }

                ui.separator();

                ui.label(format!("t = {:.2}s", self.sim.time));

                ui.add(egui::Slider::new(&mut self.sim.speed, 0.1..=6.0).text("speed"));
                ui.add(egui::Slider::new(&mut self.sim.drop_rate, 0.0..=0.65).text("drop"));
            });
        });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(430.0)
            .show(ctx, |ui| {
                self.side_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let (response, painter) = ui.allocate_painter(rect.size(), Sense::click());

            self.draw_cluster(ui, response.rect, &painter, &response);
        });
    }
}

impl RaftApp {
    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Raft state");

        ui.label("ノードをクリックすると詳細を固定表示します。右の crash/restart で障害を入れられます。");

        ui.separator();

        egui::Grid::new("nodes_grid")
            .striped(true)
            .num_columns(8)
            .show(ui, |ui| {
                ui.strong("id");
                ui.strong("role");
                ui.strong("term");
                ui.strong("vote");
                ui.strong("log");
                ui.strong("commit");
                ui.strong("counter");
                ui.strong("fault");
                ui.end_row();

                for id in 0..self.sim.nodes.len() {
                    let node = &self.sim.nodes[id];

                    ui.label(format!("S{}", id));
                    ui.label(node.role.label());
                    ui.label(node.current_term.to_string());

                    ui.label(
                        node.voted_for
                            .map(|v| format!("S{}", v))
                            .unwrap_or("-".to_owned()),
                    );

                    ui.label(node.log.len().to_string());
                    ui.label(node.commit_index.to_string());

                    ui.label(
                        node.state_machine
                            .get("counter")
                            .copied()
                            .unwrap_or(0)
                            .to_string(),
                    );

                    let button_label = if node.alive { "crash" } else { "restart" };

                    if ui.button(button_label).clicked() {
                        self.sim.toggle_node(id);
                    }

                    ui.end_row();
                }
            });

        ui.separator();

        let detail_id = self
            .sim
            .selected_node
            .or_else(|| self.sim.leader_id())
            .unwrap_or(0);

        self.node_detail(ui, detail_id);

        ui.separator();

        ui.heading("event log");

        egui::ScrollArea::vertical()
            .id_salt("events")
            .max_height(260.0)
            .stick_to_bottom(false)
            .show(ui, |ui| {
                for line in &self.sim.events {
                    ui.monospace(line);
                }
            });
    }

    fn node_detail(&self, ui: &mut egui::Ui, id: usize) {
        let Some(node) = self.sim.nodes.get(id) else {
            return;
        };

        ui.heading(format!("S{} log", id));

        ui.label(format!(
            "timeout {:.2}/{:.2}s, applied {}, alive {}",
            node.election_elapsed, node.election_timeout, node.last_applied, node.alive
        ));

        egui::Grid::new("log_grid")
            .striped(true)
            .num_columns(5)
            .show(ui, |ui| {
                ui.strong("idx");
                ui.strong("term");
                ui.strong("committed");
                ui.strong("applied");
                ui.strong("command");
                ui.end_row();

                for (i, entry) in node.log.iter().enumerate() {
                    let idx = i + 1;

                    ui.label(idx.to_string());
                    ui.label(entry.term.to_string());
                    ui.label(if idx <= node.commit_index { "✓" } else { "" });
                    ui.label(if idx <= node.last_applied { "✓" } else { "" });
                    ui.monospace(&entry.command);

                    ui.end_row();
                }
            });
    }

    // ─── クラスタ描画 ─────────────────────────────────────────────────────────
    //
    // キャンバス中央にノードを円形配置し，飛翔中のメッセージを点として描く。
    //  ─ ノード円の色: Follower=青, Candidate=黄, Leader=緑, 停止=灰。
    //  ─ 選挙タイマーの進捗を円の下にバーで表示。
    //  ─ メッセージ種別ごとに色を変え，sent_at/deliver_at から位置を補間。
    // ─────────────────────────────────────────────────────────────────────────

    fn draw_cluster(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        painter: &egui::Painter,
        response: &egui::Response,
    ) {
        let positions = node_positions(rect.shrink(50.0), self.sim.nodes.len());

        painter.rect_filled(rect, 0.0, Color32::from_gray(16));

        painter.text(
            rect.left_top() + Vec2::new(16.0, 16.0),
            Align2::LEFT_TOP,
            "Raft cluster: elections, heartbeats, log replication, commit",
            FontId::proportional(18.0),
            Color32::LIGHT_GRAY,
        );

        // ── 飛翔中メッセージの描画 ──────────────────────────────────────────
        for msg in &self.sim.messages {
            if msg.from >= positions.len() || msg.to >= positions.len() {
                continue;
            }

            let from = positions[msg.from];
            let to = positions[msg.to];

            // 線形補間で現在位置を求める。
            let denom = (msg.deliver_at - msg.sent_at).max(0.001);
            let progress = ((self.sim.time - msg.sent_at) / denom).clamp(0.0, 1.0);

            let p = from + (to - from) * progress;

            let color = match &msg.rpc {
                Rpc::RequestVote { .. } => Color32::from_rgb(235, 190, 90),

                Rpc::RequestVoteResponse { vote_granted, .. } => {
                    if *vote_granted {
                        Color32::from_rgb(120, 210, 120)
                    } else {
                        Color32::from_rgb(220, 110, 110)
                    }
                }

                Rpc::AppendEntries { entries, .. } => {
                    if entries.is_empty() {
                        Color32::from_rgb(100, 180, 255)
                    } else {
                        Color32::from_rgb(180, 140, 255)
                    }
                }

                Rpc::AppendEntriesResponse { success, .. } => {
                    if *success {
                        Color32::from_rgb(120, 230, 230)
                    } else {
                        Color32::from_rgb(250, 110, 110)
                    }
                }
            };

            painter.line_segment([from, to], Stroke::new(1.0, Color32::DARK_GRAY));
            painter.circle_filled(p, 6.0, color);

            painter.text(
                p + Vec2::new(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                msg.rpc.short_label(),
                FontId::monospace(12.0),
                color,
            );
        }

        // ── ノード円の描画 ──────────────────────────────────────────────────
        for (id, node) in self.sim.nodes.iter().enumerate() {
            let pos = positions[id];
            let radius = 52.0;

            let color = if !node.alive {
                Color32::from_gray(70)
            } else {
                match node.role {
                    Role::Follower => Color32::from_rgb(70, 115, 190),
                    Role::Candidate => Color32::from_rgb(220, 160, 55),
                    Role::Leader => Color32::from_rgb(70, 170, 95),
                }
            };

            painter.circle_filled(pos, radius, color);

            let stroke = if self.sim.selected_node == Some(id) {
                Stroke::new(4.0, Color32::WHITE)
            } else {
                Stroke::new(2.0, Color32::BLACK)
            };

            painter.circle_stroke(pos, radius, stroke);

            painter.text(
                pos + Vec2::new(0.0, -18.0),
                Align2::CENTER_CENTER,
                format!("S{}", id),
                FontId::proportional(24.0),
                Color32::WHITE,
            );

            painter.text(
                pos + Vec2::new(0.0, 6.0),
                Align2::CENTER_CENTER,
                node.role.label(),
                FontId::proportional(15.0),
                Color32::WHITE,
            );

            painter.text(
                pos + Vec2::new(0.0, 27.0),
                Align2::CENTER_CENTER,
                format!("term {} / log {}", node.current_term, node.log.len()),
                FontId::monospace(12.0),
                Color32::WHITE,
            );

            // 選挙タイマー進捗バー（Leader には不要）。
            let bar_rect = Rect::from_center_size(
                pos + Vec2::new(0.0, radius + 16.0),
                Vec2::new(radius * 1.7, 8.0),
            );

            painter.rect_stroke(
                bar_rect,
                2.0,
                Stroke::new(1.0, Color32::GRAY),
                egui::StrokeKind::Inside,
            );

            if node.role != Role::Leader && node.alive {
                let ratio = (node.election_elapsed / node.election_timeout).clamp(0.0, 1.0);

                let fill = Rect::from_min_max(
                    bar_rect.min,
                    Pos2::new(bar_rect.left() + bar_rect.width() * ratio, bar_rect.bottom()),
                );

                painter.rect_filled(fill, 2.0, Color32::LIGHT_YELLOW);
            }
        }

        // ── クリックでノード選択 ─────────────────────────────────────────────
        if response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                for (id, pos) in positions.iter().enumerate() {
                    if pointer.distance(*pos) <= 58.0 {
                        self.sim.selected_node = Some(id);
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
    }
}

// ─── ノード座標計算 ───────────────────────────────────────────────────────────
//
// 5 ノードを正五角形の頂点に配置する。
// 上が頂点になるよう -π/2 から開始する。
// ─────────────────────────────────────────────────────────────────────────────

fn node_positions(rect: Rect, count: usize) -> Vec<Pos2> {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.36;

    let mut positions = Vec::with_capacity(count);

    for i in 0..count {
        let angle = -std::f32::consts::FRAC_PI_2
            + std::f32::consts::TAU * (i as f32) / (count.max(1) as f32);

        positions.push(center + Vec2::new(angle.cos() * radius, angle.sin() * radius));
    }

    positions
}
